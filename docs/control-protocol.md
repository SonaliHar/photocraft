# Control protocol

Shape stroke defaults are drivable with `ui.set {shapeStroke: {width, align, cap, join,
miterLimit, dashes, dashOffset, opacity}}`; partial fields merge, null disables the stroke.
Values use the existing `shape.edit` names and units. Colour uses `tools.setColors`.
`ui.inspect` exposes `toolOptions.shape_stroke`, `stroke_width`, and `strokeEditor`.
Stroke Options applies valid UI edits immediately; `strokeEditor` is the open editor state,
not an uncommitted document preview. Escape/outside clicks close it without reverting edits.
See [Shape stroke options](shape-strokes.md) for limits, preview and persistence behavior.

The desktop app listens on `127.0.0.1:<port>` (loopback only). Start it with a token file so the credential is not exposed in the process command line:

```sh
photocraft --control 7878 --control-token-file /private/path/photocraft-control.token \
  --automation-read-root /work/project --automation-write-root /work/project
```

If the file does not exist, PhotoCraft creates it with a fresh 256-bit token. On Unix the new file is mode `0600`; on Windows, protect it with an appropriate user-only ACL. An existing file is reused. If neither a token nor token file is configured, PhotoCraft generates a token for that launch and writes it to standard error. `PHOTOCRAFT_CONTROL_TOKEN` and `PHOTOCRAFT_CONTROL_TOKEN_FILE` are the environment-variable equivalents.

The first request on every TCP connection must authenticate. No other method is dispatched before this succeeds:

```json
{"id": "auth", "method": "auth", "params": {"token": "<64 hexadecimal characters>"}}
```

Authenticate once per connection. A later `auth` frame on the same connection gets `{"ok": false, "error": "already authenticated"}` and the connection keeps serving.

After the successful authentication reply, each request is one JSON line:

```json
{"id": 1, "method": "ui.inspect", "params": {}}
```

Each reply is one JSON line with the same `id`:

```json
{"id": 1, "ok": true, "result": { ... }}
{"id": 2, "ok": false, "error": "unknown tool `foo`"}
```

The desktop server waits up to 60 seconds for a reply. A request still queued at that deadline is rejected before dispatch; a timeout does not cancel work that has already started. The two timeouts read differently, both starting with `timeout`: a request that was never dispatched (still queued at the deadline) says `timeout: not run` and is safe to re-send, while `timeout: no reply in 60s` means the command was dispatched and may still be running and still apply, so check the document before re-sending it. The timeout error names the likely causes: the command may still be running, or the request was queued while the window was not drawing frames — on Wayland the frame update (and with it the control drain) only runs on compositor frame callbacks, so a display that is asleep or a fully occluded window makes every request time out while the app itself is healthy.

The transport is `apps/photocraft/src/control_server.rs`, and the handlers are in `crates/ui-egui/src/control.rs`. The MCP server (`photocraft-cli mcp --bridge 127.0.0.1:<port>`, crate `photocraft-automation`) wraps this same protocol. See [MCP bridge](#mcp-bridge) below.

## Methods

- `engine.execute {command, params}`: run any engine or UI command by id. Engine commands run directly with their default params and never open a dialog. Use `ui.menu.invoke` for menu-click behaviour, which opens a command's dialog when no params are given. `params` must be a JSON object (omit it or pass `null` for none); an array, string, number or boolean is an error naming the command, in every transport (control channel, `serve`, MCP and the CLI)
- `engine.commands`: list commands with enablement; the `result` is a JSON array of command objects (see [Command list replies](#command-list-replies))
- `ui.inspect`: full UI state (tool, panels, views, dialogs, windows, window size). The menu tree is not included; use `ui.menu.list`. `brushPicker` is `null` while closed and `{pos: [x, y], size: [w, h]?, list: {collapsed, filter, renaming, showName, showStroke, showTip, scale}}` while the Brush Preset picker is open (coordinates in screen points; `size` is the picker's content size once its corner grip was dragged, `null` before that, and the drag remembers both dimensions; the three `show*` flags pick the parts of each preset's card — at least one is always on; `scale` is the footer slider, 0.15–2 with 1 the standard, multiplying the cards' width and height — the heights stop at half size and the name and tip-size text hold their 1.0 size — and dropping the tips' size numbers at 0.40 and below; the parts and the scale are remembered across restarts, while the search, collapsed groups and a rename in progress are not). `view` holds the View/Window/Type preferences: `screen_mode`, `extras`, `show` and `snap_to` flags, `flip_horizontal`, `arrange` (Window › Arrange layout), pixel aspect, font preview size, language options. `perf.timings.gpuInfo` holds the graphics adapter, backend, driver, the backend chosen at launch and why, the canvas renderer (`gpu`/`cpu`) and, after a device loss, `lost` (`help.systemInfo` returns the same as `info`)
- `ui.set {tool?, panels?, dockTabs?, dock?, dockWidth?, colorPanel?, maskTarget?, vectorMaskTarget?, selectionMode?, zoom?, center?, rotation?, fit?, theme?, brushSize?, brushSection?, brushTab?, brushesView?, brushPicker?, brushPickerName?, brushPickerStroke?, brushPickerTip?, brushPickerScale?, gradientBlendMode?, gradientClassic?, shapeStroke?}`: change UI state; any other field is an error, checked before anything changes (`zoom` is the canvas zoom where 1 = 100%, one document pixel per physical display pixel, so a scaled display doesn't magnify the image; `theme` is `pro`, `proMedium`, `studio`, `studioLight` or `classic`; `selectionMode` is the selection tools' options-bar mode, 0 New, 1 Add, 2 Subtract, 3 Intersect; `brushSection` indexes the Brush Settings sections, `brushTab` 0 = Brush Settings, 1 = Brushes; `brushesView` is `list` or `grid` for the Brushes panel; `brushPickerName`, `brushPickerStroke` and `brushPickerTip` are the Brush Preset picker's card parts (booleans, at least one on), and `brushPickerScale` its footer card-scale slider (0.15 to 2, 1 the standard); `brushPicker` is `[x, y]` in screen points to open the Brush Preset picker there (as a right-click with a painting tool does) or `null` to close it, and picking a preset in it is `tools.setBrush {"preset": name}`; `dock` is `{order: ["layers", …], heights: {"properties": 180}, collapsed: ["color"], tabOrder: {"layers": ["Paths", "Layers", "Channels"]}}`, the right-dock groups top to bottom, their heights in points, the groups collapsed to their tab strip and each group's tab order left to right by panel name (a group left out keeps its default order, unknown names are ignored and tabs not listed follow in their default order; `dockTabs` keeps indexing tabs in their default order); `dockWidth` sets the right dock width in points, clamped to 250..520; `colorPanel` is `{background}`, whether the Color panel edits the background colour; `maskTarget` targets the active layer's mask, which, like Photoshop, swaps in the foreground/background pair kept for masks (white/black until one is picked) and swaps the pixels' pair back when the pixels are targeted again)
  - `typeTransform` is `null` outside a temporary Ctrl/Cmd Type-tool drag. During a drag it reports its source document/revision, original affine, oriented `frame` (layer, local bounds, document-space corners and pivot), and captured `gesture`. Drive it with `ui.pointer` and `command: true` at `down`; `move`/`up` may release the modifier. The document/history remain unchanged during preview, and `up` applies one `type.edit` within `textEdit`'s session. Pressing Escape during the drag cancels only the preview. This pointer state is never restored from saved UI state.
- `ui.menu.invoke {id, params?, wait?}` / `ui.menu.list`: activate a menu item by id; list the menu tree. A menu item that starts a background job (a filter without a dialog, such as Blur More) replies with the job's result once it has been applied; with `"wait": false` the reply is `{job, pending: true}` at once. Window ends with the open documents as `window.document.<index>` (tab order, the active one checked, #2991); invoking one brings that document to the front and replies `{document: index}`
- `ui.set` also accepts `gradientBlendMode` (a layer blend mode name, such as `Difference`) and `gradientClassic` (boolean) for the Gradient tool options bar.
- `ui.set` also accepts `camTransformOnDrop` (boolean), the Content-Aware Move tool's Transform On Drop checkbox: a drop then puts a scale/rotate box on the patch (`ui.inspect` › `camTransform`: its document/revision, drag `offset` and `frame`), driven with `ui.pointer` (handles as Free Transform's; outside the box rotates), committed with `ui.key Enter` (one `paint.contentAwareMove` with `scaleX`, `scaleY`, `rotate`) or dropped with `Escape`.
- `ui.set` also accepts `tileGrid` (boolean), a developer overlay of the active layer's storage tiles: the tile grid over the document, every stored tile filled and labelled `tx,ty`, tiles with identical pixels in a shared colour and unique ones in blue. It has no menu item and View › Extras doesn't hide it. `ui.inspect` reports it as `view.tile_grid`.
- Each painting tool keeps its own brush, as in Photoshop: `tools.setBrush` and `ui.set {brushSize}` change the active tool's brush, and switching tools swaps in that tool's own (size, hardness, mode, opacity, flow, dynamics, smoothing). A tool used for the first time starts from the brush in hand. The colours, the erase flag and the stroke seed stay shared. So `ui.set {tool: "eraser", brushSize: 12}` sets the Eraser's size, and `ui.set {tool: "brush"}` afterwards brings the Brush's own size back.
- `ui.set` also accepts the Eyedropper options bar (#1649): `eyedropperSampleSize` (`"point"` or 1, 3, 5, 11, 31, 51, 101: the side of the averaged square, shared by the Eyedropper, a painting tool's Alt-click eyedropper, the Curves/Color Picker eyedroppers and the Info panel), `eyedropperSample` (`current`, `currentAndBelow`, `all`, `allNoAdjustments`, `currentAndBelowNoAdjustments`) and `eyedropperRing` (boolean, Show Sampling Ring). `ui.inspect` reports them under `toolOptions` (`eyedropper_size`, `eyedropper_sample`, `eyedropper_ring`). The engine command `document.sampleColor {x, y, size, sampleLayer}` returns the colour such a sample picks.
- `ui.set` also accepts the Crop tool's overlay menu (#1919): `cropOverlay` (`thirds`, `grid`, `diagonal`, `triangle`, `goldenRatio`, `goldenSpiral`), `cropOverlayShow` (`auto`: only while the crop box is dragged, Photoshop's default; `always`; `never`) and `cropOverlayOrientation` (0..3; Triangle has two orientations and Golden Spiral four, the others ignore it). `ui.inspect` reports them under `toolOptions` (`crop_overlay`, `crop_overlay_show`, `crop_overlay_orientation`). In the app, O cycles the overlay and ⇧O its orientation while the Crop tool shows a box.
  Both gradient modes use the current preset, including its opacity stops. Select `Foreground to Transparent` through `gradient.presets.select`, or pass custom `stops` and `transparency: [[0,100],[1,0]]` (location 0..1, opacity 0..100). Use `applyToLayer: false` to edit only the tool preset. Classic gradient drags dispatch `paint.gradient` without overriding them.
- `ui.set` also accepts the Crop tool's gear menu (#1919) as `cropShield`, an object patching any of `show_cropped_area` (Show Cropped Area; off hides everything outside the box behind the canvas colour, H in the app), `enabled` (Enable Crop Shield), `color` (`matchCanvas`, the pasteboard colour, or `custom`), `custom_color` (`[r, g, b]`, 0..255), `opacity` (0..100, default 75), `auto_adjust` (Auto Adjust Opacity: half the opacity while the box is dragged), `classic_mode` (Use Classic Mode, default off: the box turns and moves over the image; off is Photoshop's default mode, where the box stays upright and the view turns and pans the image behind it while the Crop tool shows a frame, P in the app; in both modes picking the tool turns a rotated view upright about its centre, Classic Mode keeping the camera upright with the frame drawn turned, and the view stays upright after a commit or cancel; Classic Mode greys out Show Cropped Area, which then acts as on, and Auto Center Preview) and `auto_center_preview` (default on: in default mode a box being resized keeps its centre where it is on screen, also after a pan, a drawn one moves to the middle of the canvas, and X re-centres it; a move never re-centres the box). `ui.inspect` reports it as `toolOptions.crop_shield`. The pending frame (`crop_rect`, `crop_angle`: its turn on the image) and the `image.crop` a commit runs are the same in both modes.
- `ui.dialog.open {kind, fields?}` (kinds `newDocument`, `about`, `layerStyle {effect?}`, `colorPicker {target: foreground|background}`, `command {command}`, `newCropPreset` / `deleteCropPreset`: the Crop tool's New / Delete Crop Preset… dialogs, fields `name` and `selected: [names]`; crop presets are `crop` tool presets, so `tool.presets.new/list/edit {"tool":"crop"}` drive them too) / `ui.dialog.set {dialog, field, value}` / `ui.dialog.confirm {dialog, wait?}` / `ui.dialog.cancel {dialog}`. Like `ui.menu.invoke`, `ui.dialog.confirm` waits for a background job its command starts (a filter dialog's OK) and replies with the result; `"wait": false` replies `{job, pending: true}` at once
  - A Layer Style color field opens the shared picker with `ui.dialog.open {"kind":"colorPicker","parent":12,"effect":"fx2","field":"color"}`. `parent` is the Layer Style dialog id; `effect` is its effect instance id. `field` must be one of that instance's color parameters (`color`, or `from`/`to` for Gradient Overlay). The picker previews over the pending style without changing the document. Picker OK changes only the parent parameter; Cancel discards the preview. Confirm the parent afterwards to apply the style. Parent confirmation is refused while its picker is open; parent cancellation closes both. The binding is exposed in `ui.inspect` as `__layerStyleColor`; changed/closed targets or a document switch invalidate it.
  - `ui.dialog.open {kind:"colorPicker", textStyle:{kind:"character"|"paragraph", id:u32}}` opens the shared Color Picker for an explicitly enabled Color attribute in Style Options. It opens the matching style panel/editor after validation; inherited colors must first be enabled through the existing style command. The picker reports its bound document/revision/style under `__textStyleColor` in `ui.inspect`. Changes preview in an isolated document; OK runs the existing style `set` command once, Cancel changes nothing. Document edits/switches or closed/changed Style Options discard the picker. `textStyle` cannot be combined with `parent`, `effect`, `field` or `target`.
  - The shell's own dialogs (Window › Workspace › New Workspace / Delete Workspace, View › Pixel Aspect Ratio › Custom Pixel Aspect Ratio…, View › Show › Show Extras Options…, View › 32-bit Preview Options…) are listed in `ui.inspect`'s `dialogs` with the id `"shell"` (at most one is open). `ui.dialog.set`, `ui.dialog.confirm` and `ui.dialog.cancel` take `{"dialog": "shell"}`; OK runs the dialog's command and keeps the dialog open if it fails
  - The unsaved-changes prompt (closing a document, Revert, quitting with unsaved work) is listed in `ui.inspect`'s `dialogs` with the id `"discard"` while it is up: `fields` gives the waiting `command` (`file.close`, `file.exit`…), the `document` it asks about, how many are `remaining`, and its `choices`. `ui.dialog.confirm` takes `{"dialog": "discard", "choice": "save" | "dontSave" | "cancel"}` (default `save`; Revert takes `dontSave` or `cancel`) and answers it as the window's buttons do, one document at a time; `ui.dialog.cancel` with `{"dialog": "discard"}` cancels it. Save of an untitled document opens the save dialog first, so the reply's `fileDialog` is true and the prompt stays open until that is answered.
  - Layer Style (`layerStyle {effect?}` opens on that effect kind): the fields hold the dialog's whole state, so agents read and drive it like a user. `effects` lists every effect instance in the layer's order as `{id, kind, on, params, fx}` (`fx` is the effect snapshot the dialog loaded; OK edits that snapshot with `params`, so anything the dialog doesn't model survives). `selected` picks the page (`blendingOptions` or an instance id), `preview` (default on) gates the live canvas preview, `p:blendingOptions` edits blend mode/opacity/fill opacity, and `patternList` names the usable patterns. OK replaces the layer's effects in one step (`layer.layerStyle.replace`); Cancel discards. "New Style…" saves the pending state via `style.presets.new {effects}`, a swatch click applies a style preset, and Make/Reset Default use `layer.layerStyle.makeDefault` / `layer.layerStyle.defaultFor`
  - Preferences (`ui.menu.invoke {id: "edit.preferences.interface"}`) exposes the sections in its `values` field. Apply (`ui.dialog.apply`) and OK (`ui.dialog.confirm`) on Preferences are refused over control because automation denies whole-preferences updates and filesystem-bearing preference sections. A refused `ui.dialog.apply` returns an error and keeps the dialog open; a refused `ui.dialog.confirm` returns an error but closes and removes the dialog. `ui.dialog.cancel` closes the dialog without saving. Individual safe preferences can instead be changed with `engine.execute` using `prefs.set` (e.g. `{"command": "prefs.set", "params": {"path": "interface.showTooltips", "value": false}}`). Settings marked for the next launch still require a restart.
  - Edit › Fill… (`ui.menu.invoke {id: "edit.fill"}`, Shift+F5, Shift+Backspace) opens the Fill dialog; its fields are `edit.fill`'s params (`contents`, `color`, `pattern`, `colorAdaptation`, `mode`, `opacity`, `preserveTransparency`), and OK remembers them in the preferences (`dialogs["edit.fill"]`).
  - Save for Web (`ui.menu.invoke {id: "file.export.saveForWebLegacy"}`) is resizable with independently scrolling settings. Dialog fields `__view` (`original|optimized|2up|4up`), `__zoom` (`null` = Fit, otherwise a factor; 1 = 100% in physical display pixels), and `__center: [u,v]` control the shared preview navigation. `__previewRects` reports pane rectangles in screen points. Drag a preview to pan, scroll/pinch to zoom at the pointer, or double-click to fit. GIF/PNG-8 expose `colorTable: ["#rrggbb", …]` (opaque source entries) and `colorTableEdits: {"0":"#rrggbb", …}` (zero-based replacements after quantization). A swatch opens a color picker; edits update the preview and saved file without remapping pixel indices or changing the document. Reset clears replacements. Changing palette/size/matte/transparency settings regenerates the table and clears index-based edits. Previews and generated tables use a bounded 2048-pixel proxy; full-size export uses that explicit table unchanged. `__paletteApproximate` reports proxy-derived palette generation, and enlarged proxy previews are labelled. The engine export command accepts the same table/edit parameters; its no-path estimate returns the source `colorTable` and `transparentIndex` as well as size statistics. Opening a new dialog does not carry a previous image's table across.
  - A pixel tool pressed on a type, shape, Smart Object or fill layer (e.g. through `ui.pointer`) opens the "Rasterize?" prompt instead of painting: a dialog with `__rasterize` (`type|shape|smartObject|fill`), `message`, `layer`, `tool` and `at`. `ui.dialog.confirm` runs the `layer.rasterize.*` command and then paints at `at` (two history states, `{"rasterized", "painted"}`); `ui.dialog.cancel` does nothing.
  - Edit › Transform › Warp on editable type (`ui.menu.invoke {"id":"edit.transform.warp"}`, or the warp toggle of an open transform box) opens Warp mode with no prompt, as in Photoshop: the mesh can't be dragged or split, only the options bar's warp style, Bend, H and V apply, and the commit sets the layer's Warp Text in one step, so the type stays editable. Directly: `engine.execute {"command":"edit.transform.warp","params":{"style":"flag","bend":40}}` on a type layer. Distort and Perspective stay greyed for type.
- `ui.window.open {document?}` / `ui.window.close {window}`: extra document windows
- `ui.pointer {events: [{kind: down|move|up, x, y, pressure?, tiltX?, tiltY?, rotation?}], tool?, modifiers?, button?}`: drive the active tool in document coordinates (pressure 0..1, tilt in degrees -90..90, barrel rotation 0..360: a simulated pen). Modifier flags (`shift`, `alt`, `command`, `ctrl`, `space`) go either at the top level or grouped under `modifiers`; when `modifiers` is present, top-level flags are ignored. `space: true` holds Space (the Crop frame, marquee, lasso or shape being drawn then moves instead of growing). With the Crop tool in its default mode (Use Classic Mode off) a drag inside the box moves the image and one outside turns it, the view panning and turning with it; a drag's `move`/`up` points are read as the view showed the document at its `down`, so an `up` where the last `move` was changes nothing more. `command: true` with a painting, retouching, eraser, gradient, eyedropper or other non-selection tool makes the gesture the Move tool's, as holding ⌘ (Ctrl off the Mac) does in the app: a drag moves the active layer (the selected pixels when there is a selection), with `alt: true` as well it duplicates the layer first, and a click picks the layer under the point; the tool is unchanged afterwards. The Hand, Rotate View, Zoom, Crop, Slice, Path Selection, Pen, shape, Type and selection tools keep `command` for themselves. `ctrl: true` with `alt: true` and a brush tool is the on-canvas brush resize instead (in the app, Ctrl+Alt-drag off the Mac). `button: "right"` (or `"secondary"`) is the right button: with the Move tool (or `command: true` with any tool) it opens the canvas layer menu (`layerMenu` in `ui.inspect` lists the layers there, topmost first; select one with `layer.select`); with Marquee, Lasso, Magic Wand, Object Selection, or Pen it opens a tool context menu (`canvasToolMenu` in `ui.inspect`, with ordered command ids, separators, and enabled states); with Zoom or Hand it opens the view menu there (Zoom In and Zoom Out step about the pressed point; `view.fillScreen` is its Fill Screen row); with a painting tool it opens the Brush Preset picker, or erases with Preferences › Tools › Right-click with painting tools = erase. The menus open at the screen position of the pressed document point. Use `ui.context.choose {id}` for enabled tool-context actions. The Pen menu is documented in `docs/context-menu-parity.md`. `button` is `left` (default), `right`, `secondary` or `middle`, and an event's `kind` is `down`, `move` (default) or `up`. With a Color Picker as the top dialog, `down` and `move` sample the image into its new colour instead (its eyedropper, like a click on the canvas) and the tool is not driven. A `down`/`up` at the same point with the Rectangle, Ellipse, Triangle or Polygon tool opens Create Rectangle / Ellipse / Triangle / Polygon (Photoshop's click-to-size dialog); set its `width` and `height` (document pixels), `fromCenter`, `radii` [tl, tr, br, bl] (Rectangle) or `sides` and `starRatio` (percent, Polygon) with `ui.dialog.set`, then `ui.dialog.confirm` runs `shape.create` at the clicked point.
  - Magnetic Lasso (`tool: "magneticLasso"`): a `down`/`up` pair is a click. The first sets the first fastening point (its modifiers set the selection mode); later clicks fasten a point, or close the border on the first point. `move` events trace the border with or without a button held, fastening points by themselves; with `alt: true` a click draws a straight segment and a drag a freehand one. `ui.inspect` shows the border in progress under `magnetic` (`path`, `anchors`, `live`, `mode`). `ui.key` Enter closes it along the edges, Backspace removes the last fastening point, Escape cancels. To select along edges in one call instead, use the `select.magneticLasso` command with rough points around the shape.
  - Levels or Curves adjustment layer, Properties eyedropper armed (`layerPicker` in `ui.inspect`: `{layer, point, tool}`, set by the panel's Set Black/Neutral Gray/White Point buttons): a `down` sets that point from the image below the layer as one `layer.setAdjustment {"layer", "eyedropper": {"point", "at": [x, y], "size"}}` (send that command directly to skip the UI) and the tool is not driven; Esc, another tool or another layer disarms it.
- `ui.click {x, y, button?, count?}` / `ui.move {x, y}`: synthetic pointer input in screen points (`button`: `left` (default), `right` (or `secondary`) or `middle`). `count` (default 1, `2` for a double-click) is at most 256; a larger one is an error and queues nothing
- `ui.key {key, command?, shift?, alt?, ctrl?}` (flags may also be grouped under `modifiers`): press and release a key, e.g. `{"key": "ArrowLeft", "shift": true}`
- `ui.pointer`, `ui.click`, `ui.move`, `ui.key` and `ui.screenshot` validate every parameter before acting, as `ui.set` does: an unknown field (top-level, under `modifiers` or in an event), a value of the wrong JSON type (`"x": "abc"`, `"ctrl": "yes"`), a `button`, `kind`, `tool` or `key` outside the accepted set, or a number outside its range (`pressure`, tilt, `rotation`, `count`) is an error naming the parameter, and nothing is queued or applied. A `null` field counts as omitted and takes its default
- Engine commands (`engine.execute`, `photocraft-cli run`, MCP `command_run`) reject a param key their `params` doc does not name for the commands listed in `STRICT` (`crates/engine/src/param_check.rs`, today `layer.hideLayers` and `layer.showLayers`): `{"ids":[4]}` is the error `unknown parameter \`ids\` (expected: layer)`, and nothing runs. Other commands still ignore unknown keys until their `params` doc is complete and they are added to that list (`coalesce` and `target` are always accepted).
- `ui.type {text}`: type text (goes to the focused widget, or to the canvas while the Type tool is editing)
- `ui.resize {width, height}`: resize the main window
- `ui.gpu.simulateLoss {error?}`: act as if the wgpu device was lost (or reported an error with `error: true`). The app switches to the CPU image compositor for the rest of the session and shows the same recovery warning as on a real loss. `gpuFallbackNotice` in `ui.inspect` contains the reason while the warning is open. Keep Using CPU saves CPU compatibility for the next launch; Retry GPU saves GPU mode and asks the user to save and restart. The window renderer still requires a working graphics or software adapter. Returns `wasActive` and `gpuInfo`. For testing the fallback
- `ui.screenshot {path?, focus?}`: capture the main window (PNG). With no path the reply contains
  base64 PNG data; a path is relative to the automation write root. Raises the window first
  (default) because occluded macOS windows stop rendering
- `ui.focus`: bring the main window to the front
- `app.open {path}` / `app.save {path?, tgaBits?}`: relative file I/O through the configured automation roots (`app.open` reads under the read root, `app.save` writes under the write root; absolute paths, `..` and paths escaping the root are refused, and both fail closed when no root was granted). Both reply with `warnings` (import/export notes such as "adjustment layer flattened"; `[]` when none), also shown to the user in the status bar and as a notice (`notices` in `ui.inspect`); `app.open` also returns the `path` and document `name` (directory `.pcraft` — the native on-disk form — loads through the read root's capability; the single-file (zip) form takes the bytes path), `app.save` the `path` written. `app.save` without `path` writes back only to the document's own PSD, PSB or `.pcraft` file, like File › Save. `tgaBits` (24 or 32) sets a TGA's bits per pixel: 32 writes the alpha channel (or else the transparency) as its alpha, 24 none; without it, 32 with an alpha channel or transparency, else 24. Automation saves never show TGA or TIFF options. Automation opens and saves never fire script events. Use these two rather than `file.open`, `file.save`, `file.saveAs` or `file.saveACopy`, which the control channel refuses (see [Engine commands](#engine-commands))
- `app.quit`

Trusted local `file.open` callers may explicitly pass
`{"path":"chosen.pcraft","nativeProfilePolicy":"keepCurrent"}`. This optional
native-only lane retains the saved current ICC/encoding/working interpretation
without applying Color Settings or opening its profile question. Omitting
`nativeProfilePolicy` preserves ordinary Open policy, including profile prompts
and conversion. With the explicit option but no `path`, the interactive picker
accepts one `.pcraft` path, or a bounded byte upload when the dedicated browser
capability is installed. Other policies, malformed parameters,
unavailable services, unsupported models and quota failures are
errors; they never fall back to ordinary import. The platform must supply the
actual authorized reader, and retained source/resource locators grant no reads.
Control/MCP `file.open` remains denied. Capability-scoped `app.open` may explicitly
supply the same policy with a required `.pcraft` path; it uses a separately
installed held-read-root service and never falls back to the interactive reader
or a chooser. Its reply is `{path,name,warnings:[]}`. Missing rooted capability
is an error. Automation retains its suppression of recent files and script events.
Both explicit routes retain the persisted document title; the reply name and
runtime source label identify the actual input basename independently.

Explicit owned original-XCF Open uses the separate
`{"path":"chosen.xcf","xcfProfilePolicy":"keepCurrent"}` option on trusted local
`file.open`, or on held-read-root `app.open`. The source-selected reader supports
the existing staged plain implicit nonlinear RGB/Gray raster class and the
separate full-canvas Threshold Alpha class. A distinct Histogram Threshold lane
accepts the existing RGB nonlinear 8x6 U16/F32/F64 source class with its exact
three arguments and related empty filter mask. This is not a fallback
for other XCF effects, masks, groups, channels or source profiles. It keeps
the importer's current interpretation without applying Color Settings. Omitting
the key preserves ordinary Open and its questions/conversions. Combining this
key with `nativeProfilePolicy`, unknown policies or extra fields is an error.
Interactive callers may omit `path` to choose one `.xcf`; scoped callers must
supply a path and an independently installed held-root callback. Neither route
falls back to another reader/importer on failure. Input/source capture and model
ownership share the real existing document-catalog quota. The original remains
source-protected/read-only; no saved locator becomes read authority. Replies
include actual `warnings`, `assessment` (or null), and `sourceReadOnly`; scoped
replies also include `path` and basename `name`. These import-time diagnostics
are not a claim about later edits or unqualified source/render families.

With the browser byte-picker capability installed, interactive
`file.open {"xcfProfilePolicy":"keepCurrent"}` instead opens a dedicated picker.
The actual catalog quota covers the provider buffer and immutable Rust capture
before reading, with a 16 MiB input ceiling and bounded copies. Escape cancels
the host read. The selected name is only a label: no path or read authority is
installed, and ordinary browser picker/drop behavior is unchanged.
The separate `file.open {"nativeProfilePolicy":"keepCurrent"}` browser picker
uses the same input ceiling and capture lifecycle. It keeps the native archive's
saved title and current interpretation; its label grants no filesystem path.
Native archive reads retain the upload's actual quota owners and use the future
operation's cancellation/work control after the picker has closed.

## Engine commands

Change the UI language without restarting with `prefs.set`:
`{"path":"interface.language","value":"fr"}`. Supported codes and preview/apply behaviour
are documented in [UI localisation](localization.md). `prefs.get` and the existing preference
store expose and persist the same setting; scripts keep using canonical command IDs.

`engine.execute` runs any command by id. `engine.commands` (or the engine command `command.list`) lists them all, with labels, menu paths, shortcuts, a parameter description, and whether each is currently enabled (reply shape: [Command list replies](#command-list-replies)). Each row carries `enabled` (bool), `disabledReason` (`null` when enabled, otherwise the precondition that failed, such as `"no document open"`, `"no selection"` or `"the clipboard is empty"`) and `needsDocument` (true when the command is off because no document is open, so the first step is to create or open one; a command may still have its own precondition after that, such as an active text layer. False when enabled or off for another reason). With no document open, nearly every document command reports `enabled: false, needsDocument: true`; plan around `needsDocument` rather than reading `enabled` alone (#3664). Examples:

| Command | Params |
|---|---|
| `file.new` | `{"width":1920,"height":1080,"mode":"rgb","depth":8,"background":"white"}` |
| `layer.new.layer` | `{"name":"Ink"}` |
| `layer.select` | `{"layer":id,"mode":"replace\|toggle\|range\|add"}`: ⌘-click = toggle, ⇧-click = range; `document.inspect` reports `selectedLayers` and a per-layer `selected` flag |
| `channel.target` / `channel.setVisible` | Channels panel target and eyes (`"composite"`, colour name, alpha index or name, `"quickMask"`). `document.inspect` (and `channel.list`) report `channels`: composite/colour/alpha rows with visibility, channel options, `quickMask`, `target`. Pixel commands without a `target` param follow the targeted channel |
| `layer.setProps` | `{"layer":id?,"name":…,"visible":…,"opacity":0..1,"blend":"Multiply"}` |
| `layer.fill.solidColor.set` | `{"layer":id?,"document":id?,"color":"#rrggbb"}`: edit a Solid Color Fill in one undo step; RGB without alpha preserves the fill's alpha. Float RGB arrays and a stored `Color` are also accepted. `document` rejects stale dialog targets. |
| `layer.fill.pattern.set` | `{"layer":id?,"document":id?,"pattern":id\|name?,"scale":1..1000?,"angle":-180..180?,"link":bool?,"phase":[x,y]?}`: edit a Pattern Fill (the Pattern Fill dialog's OK) in one undo step; omitted values keep the layer's, `phase` `[0,0]` is Snap to Origin. `layer.newFillLayer.pattern` from a menu (no params) and `layer.layerContentOptions` on a Pattern Fill layer open the Pattern Fill dialog. |
| `layer.layerMask.edit` | `{"layer":id?,"document":id?,"density":0..100?,"feather":0..1000?}`: edit pixel-mask Density (%) and Feather (px) without changing mask pixels, including adjustment masks. At least one setting is required; `document` rejects stale targets. A shared `coalesce` key groups slider updates into one undo step. |
| `layer.setExpanded` | `{"layer":id?,"expanded":bool?,"all":bool?}`: open/close a group in the Layers panel (no `expanded`: toggle; `all`: every group). A view change saved with the document (PSD open/closed folder), not an undo step; `document.inspect` reports `expanded` on groups |
| `layer.newAdjustmentLayer.hueSaturation` | `{"hue":30,"saturation":10}` |
| `paint.stroke` | `{"points":[[x,y,pressure],…],"size":20,"color":"#ff0000"}` |
| `select.rect` | `{"x":0,"y":0,"width":100,"height":50,"mode":"add","ellipse":false}` |
| `document.inspect` | `{}`: layer tree, history, snapshots (`[{id, name}]`, for `history.selectSnapshot`), selection bounds |
| `document.pixel` | `{"x":10,"y":10}`: composite RGBA |
| `document.presets.list` | `{}` → `{presets:[{name,settings}]}`: saved New Document configurations |
| `document.presets.save` | `{"name":"Product square","settings":{"width":1600,"height":1600,"background":"transparent"}}`: save a snapshot; names are trimmed, 1–255 characters, and ASCII case-insensitive duplicates are rejected |
| `document.presets.get` | `{"name":"Product square"}` → `{preset,params}`: fetch settings; pass `params` to `file.new` to create a document, optionally adding a document `name` |
| `document.presets.delete` | `{"name":"Product square"}`: delete the named saved configuration |
| `document.recent.list` | `{}` → `{recent:[settings]}`: File › New › Recent, the settings of the documents last created with the New Document dialog, newest first (at most 20) |
| `document.recent.add` | `{"settings":{…as document.presets.save}}`: put settings first in Recent (the dialog does this after Create; equal entries collapse); not recorded into actions |
| `type.hitTest` | `{"layer":id?,"x":px,"y":px}`: character under a document point. No `layer` picks the topmost visible type layer there. Result `{"layer","index","line","inside"}` |
| `type.caret` | `{"layer":id?,"index":char}`: caret segment in document pixels, `{"index","line","segment":[[x,y],[x,y]]}` (rotated and vertical type included) |
| `type.navigate` | `{"layer":id?,"index":char,"move":"wordPrev\|wordNext\|linePrev\|lineNext\|lineStart\|lineEnd\|start\|end","x":px?}`: neighbouring caret. `x` keeps the column across line moves. Result `{"index"}` |
| `actions.list` | `{}` → `{actions:[{name, steps}], recording}` (the name being recorded, or null) |
| `actions.get` | `{"action": name or index}` → `{name, steps:[[id, params], …], disabled:[step, …]}`. `steps` is the shape `file.automate.batch` and droplets take; `disabled` lists the 0-based steps playback skips |
| `actions.record` | `{"name":"Red"}` starts a new action (default name `Action N`). `{"action": name or index}` appends to one that exists |
| `actions.stop` | `{}` → `{action, steps}`. Copies replayable journal entries since `actions.record` (queries and action-editing commands omitted) |
| `actions.play` | `{"action": name or index, "from": step?}`. `from` and `failed.step` are 0-based. Returns `{action, ran, failed?:{step, id, error}}` and still returns ok when a step fails, so a partial run is reported. Leaves one history step per step that ran. Supports nested action calls up to 16 levels, rejecting cycles. While recording, playing another action appends one named `actions.play` step instead of its expanded commands. On an untrusted session each step is authorized the same way as a top-level command |
| `actions.move` | `{"action": name or index, "step": index?, "to": index}` moves a step within its action, or moves the whole action when `step` is omitted. `to` is the final zero-based index. Pending steps can be reordered while recording; invalid moves leave the list unchanged. |
| `actions.delete` | `{"action": name or index, "step": index?}`. With a zero-based `step`: `{action, deletedStep, steps}`, also allowed while recording. Without `step`: `{deleted}`, refused while recording |
| `actions.toggle` | `{"action": name or index, "step": index?, "enabled": bool?}` → `{action, disabled}`. Switches one 0-based step (pending recorded steps included) on or off; omitting `enabled` flips it. Without `step`, switches every step: on unless all already are. Playback, Batch and droplets skip steps that are off. In the desktop panel, click a step's check box |
| `actions.rename` | `{"action": name or index, "name": str}` → `{action, index}`. Empty, whitespace-only or duplicate names are rejected and keep the old name. Nested `actions.play` steps that call the action by name and its function key follow the new name. In the desktop panel, double-click an action's name to rename it in place (Enter commits, Esc cancels) |

The Actions list fills the resizable dock group.

The Actions panel shows replayable steps immediately while recording. Click an individual
step to select it, then use the trash button to remove that step; selecting the action
heading instead targets the whole action. Step deletion also works during recording
and does not undo the document edit. The control equivalent is
`actions.delete {"action": "Name", "step": 0}` (zero-based). Omitting `step` deletes
the whole action, which still requires recording to be stopped.

Desktop actions also record and replay View › Fit on Screen (`view.fitOnScreen`),
100% (`view.actualPixels`), Zoom In and Zoom Out through their menu commands or shortcuts.
Select an action and press Record to append steps; the plus button starts a new action.
A final `["view.fitOnScreen", {}]` step fits the resized document when the canvas is laid out.
Headless playback reports unsupported view commands as failed steps.

In the desktop Actions panel, right-click an action to assign a function key (F1–F12),
or choose None to remove it. The binding follows the action name, regardless of the selected
row, and is saved in preferences. For example, the existing `edit.keyboardShortcuts` command
accepts `{"set":{"actions.play:My action":"F6"},"allowUnknown":true,"removeConflicts":false}`.
A named action takes precedence over the ordinary menu shortcut while it exists and is bound;
removing its binding or deleting it restores that menu shortcut. The panel moves a key away
from another action when assigning it. Action playback still checks each nested command's
automation permissions.

UI-level commands (`view.zoomIn`, `window.theme.pro`, `edit.search`, …) are also accepted by `engine.execute` and `ui.menu.invoke`.

Commands that read or write files by path, instead of through the automation roots, are refused with "automation command `…` uses ambient filesystem paths and is disabled; use capability-scoped document methods". That covers every `file.*` command except `file.new`, the `file.close*` commands and a few path-free ones such as `file.fileInfo`, so `file.open`, `file.save`, `file.saveAs` and `file.saveACopy` always fail here: open and save with `app.open` / `app.save`. Path parameters of other commands (`layer.exportAs {path}`, `filter.distort.displace {mapPath}`, preset imports, and plug-in install/reload) are refused the same way. `prefs.set` rejects whole-preference updates and file-backed sections (`colorSettings`, `scriptEvents`, `historyLog`, `plugIns` and `scratchDisks`) so automation cannot configure ambient file access indirectly. The desktop app also refuses `image.mode.*`, which can load the colour profiles set in its preferences. The rules are in `crates/automation/src/workspace.rs`.

Embedded Smart Objects can be opened with `layer.smartObjects.editContents {layer?}` or
unpacked with `layer.smartObjects.convertToLayers {layer?}`. This includes Photoshop sources
stored by UUID in the document's embedded linked-layer blocks. Sources requiring a disk read
remain refused, including nested linked objects that need re-rendering during unpacking.
In the opened contents document, use `layer.smartObjects.saveContents {}` to update the parent
in memory, or close the dirty contents document. For nested objects, save from the deepest
contents outward. `app.save` exports the active document to a scoped file; it is not a substitute
for Save Contents. Save the parent document to disk when the edits are complete. These rules
also apply to recorded actions and synthetic UI input; interactive desktop commands retain
their usual file access. Source replacement, relinking and export commands remain restricted.

`type.editText` starts inline editing of the active type layer and selects all its text, like
double-clicking its thumbnail in Layers. Text input, Commit and Cancel use the existing Type tool
editing session; non-type layers return an error without changing the tool or document.

### Command list replies

`engine.commands` (control channel and `serve`), the engine command `command.list` and the MCP
tool `command_list` all reply with a bare JSON array, not an object: there is no
`{"commands": [...]}` wrapper. On the control channel and `serve` the array is the reply's
`result` (`{"id":1,"ok":true,"result":[…]}`); the MCP tool returns it as its JSON text.
Each element describes one command, in registry order:

```json
{"id":"file.new","label":"New…","menu":["File"],"shortcut":"Cmd+N","params":"{\"width\":1..300000=1920,…}","enabled":true,"disabledReason":null,"needsDocument":false}
```

| Key | Type | Meaning |
|---|---|---|
| `id` | string | The id to pass to `engine.execute` / `command_run` |
| `label` | string | Menu label |
| `menu` | array of strings | Menu path above the item (`[]` for commands without a menu item) |
| `shortcut` | string or `null` | Default shortcut, e.g. `"Cmd+Shift+X"` |
| `params` | string | Human-readable parameter doc (not a JSON schema) |
| `enabled` | boolean | Whether the command can run in the current state |
| `disabledReason` | string or `null` | Why the command can't run now (`null` when `enabled`) |
| `needsDocument` | boolean | `true` when it is off because no document is open (#3664) |

Filtering narrows the same array without changing its elements: `serve`'s `engine.commands
{filter}` and the MCP tool's `filter` keep commands whose id or label contains the text
(case-insensitive), and the MCP tool's `enabled_only` keeps those with `enabled: true`. The
desktop control channel's `engine.commands` takes no filter and always returns every command.
The shape is pinned by a test in `crates/automation/src/rpc.rs` (#3684).

### Background jobs (#210)

Long commands (every `filter.*`, `edit.contentAwareFill`, `edit.contentAwareScale`, `file.automate.photomerge`, `brush.presets.importAbr`), file opens and interactive saves (File › Save, Save As and the TIFF Options continuation, #2017) run as background jobs in the desktop app: the window keeps drawing, the status bar shows progress with a Cancel button, and jobs that lock the active document show a modal progress dialog (Esc cancels). `engine.execute`, `ui.menu.invoke`, `ui.dialog.confirm` and `app.save` still wait for the result by default; pass `"wait": false` to get `{job, pending: true}` at once, then poll `jobs.list` (`state`: running, done, failed, cancelled; `progress` 0–1; the result or error) and stop it with `jobs.cancel {job}`. A cancelled or failed job leaves the document unchanged. While a job runs, commands that would edit its document fail with "… is still running on this document". `ui.inspect` reports `jobs` (running jobs, opening files, saving files). Set `PHOTOCRAFT_INLINE_JOBS=1` to run everything inline.

## Preferences

Preferences (Edit › Preferences, grouped like Photoshop's dialog sections) live in the engine, so
the same commands work in the app, the CLI and headless MCP. Paths are dotted camelCase keys:
`<section>.<key>`, with sections `general`, `interface`, `workspace`, `tools`, `historyLog`,
`fileHandling`, `export`, `performance`, `scratchDisks`, `cursors`, `transparencyAndGamut`,
`unitsAndRulers`, `guidesGridAndSlices`, `plugIns`, `type`, `enhancedControls`, `rawDefaults`,
`integrations`, plus `shortcuts.<command id>`, `menus` (`hidden`, `colors.<id>`), `toolbar`
(`hidden`, `order`) and `colorSettings` (Edit › Color Settings).

| Command | Params |
|---|---|
| `prefs.get` | `{"path":"performance.historyStates"}`; no path returns everything |
| `prefs.set` | `{"path":"cursors.painting","value":"precise"}` or `{"values":{"unitsAndRulers.rulers":"cm","guidesGridAndSlices.gridColor":"#ff8800"}}`. Values are validated (choices, ranges, `#rrggbb` colours, shortcut syntax); a batch applies all or nothing. A section path takes an object and merges it key by key |
| `prefs.reset` | `{"path":"performance"}` (a section or key); no path resets everything. An individual `shortcuts.<id>` or `menus.colors.<id>` reset removes the override and returns `null`, including when it is already absent |
| `edit.preferences.<section>` | the section's values; in the app (no params) it opens the Preferences dialog on that section |
| `edit.keyboardShortcuts` | `{"set":{"edit.fill":"Cmd+Shift+F"},"reset":true\|["id",…],"removeConflicts":true,"filter":"blur","list":false,"importKys":"<.kys XML>"}`: returns overrides, matching commands and conflicts. A shortcut moved to another command is removed from its old owner unless `removeConflicts` is false. `""` removes a shortcut, `null` restores the default. The held temporary tools are bindable too: `tools.temporary.hand` (Space; also repositions a selection being drawn), `tools.temporary.zoomIn` (Cmd+Space), `tools.temporary.zoomOut` (Alt+Space; Alt added to a held Zoom In, Cmd+Alt+Space, zooms out too), `tools.temporary.erase` (`` ` ``; with the Brush or Pencil, a stroke started while it is held is in Clear mode, `paint.stroke`/`paint.pencil` `"mode": "clear"`, erasing with that tool's brush); they list with `"hold": true`. ⌘ (Ctrl off the Mac) held alone is the Move tool with the painting, retouching and other non-selection tools, as in Photoshop; it is a modifier, not a binding. `importKys` applies a Photoshop `.kys` set (the file's XML text, at most 4 MiB): rows are matched to commands by menu label, a key equal to the default restores it, keys also given in `set` win, tool keys are not imported, and the result gains `import` (`name`, `imported`, `set`, `unknown`, `unreadable`, `alternates`, `toolKeys`). In the app it opens Keyboard Shortcuts and Menus, whose Import Shortcuts… button loads a `.kys` set into the dialog (opening or dropping a `.kys`, or Photoshop's live `Keyboard Shortcuts.psp`, does the same). On a desktop with Photoshop installed, the first launch that has no shortcut overrides yet asks whether to import the newest install's live set (Import / Don't Import; `ui.inspect` shows it as `kysOffer`); the answer is recorded in the preferences as `dialogs.photoshopShortcuts`, so it is asked once |
| `edit.menus` / `edit.toolbar` | `{"hide":["edit.fade"],"show":[…],"color":{"edit.fill":"red"},"reset":false}` / `{"hidden":["Sponge"],"order":[…]}` |
| `edit.colorSettings` | `{"workingRgb":"display-p3","workingCmyk":"coated-cmyk","workingGray":"sgray","policyRgb":"preserve\|convert\|off",…,"askOnMismatch":true,"intent":"perceptual","bpc":true}`; honoured when opening files (`file.openAs`, the app's File › Open) and by Image › Mode and "working" profile specs. `color.profileMismatch {"action":"preserve\|convert\|discard\|assignWorking"}` answers the mismatch prompt |

Values the app honours live: history states (every open document), effect-cache budget, interface
theme, canvas colour and border, checkerboard size and colours (CPU and GPU canvas), gamut warning
colour and opacity, guide/grid/smart-guide colours and styles, grid spacing and subdivisions, ruler
units (rulers, Info panel, Image Size default unit), image interpolation (Image Size default),
painting and other cursors, zoom with scroll wheel, Use Shift Key for Tool Switch, keyboard
shortcuts, hidden and coloured menu items, autosave interval and crash recovery, and the history
log text file. GPU on/off and the GPU tile size apply at the next launch.

The desktop app stores them in `preferences.json` in the platform config directory (macOS
`~/Library/Application Support/Photocraft`, Windows `%APPDATA%\Photocraft`, Linux
`$XDG_CONFIG_HOME/photocraft`; override with `PHOTOCRAFT_CONFIG_DIR`); autosaves go to its
`Recovery` folder, and the window size and position, the dock width and floating-panel geometry
to `ui.ron` beside it (a file holding values that would crash startup is discarded). In portable mode (a `portable.txt` or `PhotoCraft.portable` file beside the
executable, as in the Windows portable zip) that directory is `PhotoCraftData` next to the
executable instead. The web build keeps them in `localStorage`. A save writes only the values
this instance changed since it last loaded or saved them over what storage holds now, so a second
browser tab (or app window) never reverts the other's changes; for a value changed in both, the
latest save wins. A failed write stays pending and is retried (after 2 s, doubling up to 30 s, and
sooner after a further change); the status bar reports the first failure and a notice a
persistent one.

User and imported (`.abr`) brush presets live in the config directory's `Presets` folder: one
`.pcbrushes` JSON file per preset group, content-addressed tip bitmaps under `tips/`, and an
`index.json` with the group order and deleted built-ins (see `photocraft_engine::preset_store`).
The store loads in the background at launch and syncs after every brush preset change; built-ins
are never written. The Actions list is `actions.json` in that same folder and survives a restart.
Headless CLI/MCP sessions and the web build keep brush presets and the action list for the session
only, unless a store is attached. Gradient presets (including imported `.grd` groups) persist with
the preferences.

Named New Document presets also persist with preferences (`presets.documents`) on desktop and
web. File › New › Save Preset… saves the current form; the Saved tab selects a configuration or
deletes it. Selection only fills the form: Create still runs `file.new`, and the document name
is independent of the preset name. Settings include pixel width/height, resolution in ppi,
mode, depth, background, and display `unit` (`px`, `in`, `cm`, `mm`, `pt`, `pica`) and
`resolutionUnit` (`in`, `cm`). The defaults match File › New. Background Color captures the
toolbox colour as `backgroundColor: [r,g,b]` (0..1), so reusing it does not follow later toolbox
changes. Up to 256 presets are kept. Invalid saved entries are skipped independently; other
presets and preferences still load. Automatic Recent configurations are separate follow-up work.

## Snapping

With View › Snap on, tool gestures snap to the View › Snap To targets (guides and grid while they
are shown, layer edges and centres, document bounds and centre, selection edges) within 8 screen
pixels: Move tool drags (the moved layers' bounds), marquee, crop, shape, type-box and pen points,
Free Transform handles and drags inside the box, and guides. Holding Ctrl disables snapping for
the drag. With View › Show › Smart Guides on, the Move tool also snaps to other layers and draws
magenta alignment lines. `ui.pointer` drives the same code, so agents get identical results.

## MCP bridge

`photocraft-automation` provides an MCP server built on the official Rust SDK (`rmcp`). It runs in one of two modes:

- **Headless** (`photocraft-cli mcp`): an in-process `photocraft_engine::Session`. There is no window.
- **Bridge** (`photocraft-cli mcp --bridge 127.0.0.1:7878 --control-token-file <path>`): every tool is forwarded to a running `photocraft --control 7878 --control-token-file <path>` over this protocol, so agents see and drive the live app.

The bridge keeps one authenticated TCP connection open, and drops and reconnects before a call once the connection has been idle long enough that the app (which closes idle connections after 30s) may have dropped it. A transport failure while sending a request or waiting for its reply, including a reply timeout, drops the connection and reports that the operation may have completed; inspect the document before retrying an edit. Edits are never automatically resent (#1007); a connection that could not be established is reported as "the request was not sent" instead. The MCP server opts its pure-read tools (`ui_inspect`, `doc_inspect`, `command_list`, `jobs_list`, `session_list`, `ui_screenshot`) into one automatic retry on a fresh connection, since a read applies nothing; clients driving `BridgeClient` directly opt in per call with `call_read`. The next separate call reconnects and authenticates, and reply lines whose `id` does not match the request are skipped. The client's call timeout is 55s, strictly below the server's 60s reply deadline, so the bridge's error is the one callers see; commands that can run longer should pass `"wait": false` and poll `jobs.list`. It only accepts loopback addresses, because the app only listens on loopback. Supply its bearer token with `--control-token-file`, `--control-token`, `PHOTOCRAFT_CONTROL_TOKEN_FILE`, or `PHOTOCRAFT_CONTROL_TOKEN`:

```sh
photocraft-cli mcp --bridge 127.0.0.1:7878 \
  --control-token-file /private/path/photocraft-control.token
```

How each MCP tool maps onto control methods in bridge mode:

| MCP tool | Control method |
|---|---|
| `command_run {id, params, wait?}` | `engine.execute {command: id, params, wait}` |
| `jobs_list` / `jobs_cancel {job?}` | `jobs.list` / `jobs.cancel {job?}` |
| `command_list {filter?, enabled_only?}` | `engine.commands` (filtered by the MCP server; replies with the same bare array, rows keep `disabledReason` and `needsDocument`, see [Command list replies](#command-list-replies)) |
| `doc_new {…}` | `engine.execute {command: "file.new", params}` |
| `doc_inspect` | `engine.execute {command: "document.inspect"}` |
| `doc_open {path, xcfProfilePolicy?, nativeProfilePolicy?}` | `app.open {path, xcfProfilePolicy?, nativeProfilePolicy?}` |
| `doc_place {path, …}` | `engine.execute {command: "file.placeEmbedded", params}` |
| `doc_export_layers {dir, …}` | `engine.execute {command: "file.export.layersToFiles", params}` |
| `doc_export_data_sets {dir, …}` | `engine.execute {command: "file.export.dataSetsAsFiles", params}` |
| `doc_import_data_sets {path, …}` | `engine.execute {command: "file.import.variableDataSets", params}` |
| `doc_save {path?, tgaBits?}` / `doc_export {path, tgaBits?}` | `app.save {path?, tgaBits?}` |
| `doc_render_preview {max_side?}` | `ui.screenshot`, returned directly as PNG image content |
| `session_list`, `ui_inspect` | `ui.inspect` |
| `ui_screenshot {max_side?}` | `ui.screenshot`, returned as PNG image content |
| `ui_pointer {events, modifiers?, button?}` | `ui.pointer` (other arguments are an error) |
| `ui_menu_invoke {id}` | `ui.menu.invoke` |
| `ui_set {fields}` | `ui.set` |
| `control_call {method, params}` | any method, passed through unchanged |

Bridge previews capture the app window; passing `index` is an error. Headless `doc_render_preview` supports `index` without changing the active document.

`doc_select` and `doc_close` work only in headless mode. The `ui_*` tools and `control_call` work only in bridge mode; in headless mode they return a tool error that explains how to start bridge mode.

**Security note:** TCP control uses a bearer token, not client identity or general per-method
authorization. A client that possesses the token receives the non-filesystem control surface,
including UI input, command execution, and application control. Keep token files private, do not
commit or log tokens, and do not pass a token directly on a shared system where process command
lines are visible. The protocol is unencrypted and must remain on loopback; do not tunnel or proxy
it to an untrusted host.

Filesystem access fails closed unless launch-time read and/or write roots are granted with
`--automation-read-root` and `--automation-write-root`. The environment-variable equivalents
(`PHOTOCRAFT_AUTOMATION_READ_ROOT` / `PHOTOCRAFT_AUTOMATION_WRITE_ROOT`) are supported by
the desktop `photocraft` process, but not by headless `photocraft-cli mcp` or `serve`;
for those CLI modes, supply the flags explicitly. In MCP bridge mode, configure roots on the
desktop process rather than on the bridging CLI. Request paths must be
non-empty, forward-slash relative paths beneath the applicable root, or absolute paths that lie
beneath it. An absolute path is compared with the root as text and the rest is checked like a
relative path; nothing is resolved on the filesystem. Absolute paths outside the root, parent
traversal, alternate separators in relative paths, drive/device/stream prefixes, malformed
components, and link escapes are rejected before file effects. Read and write authority are
independent; the parent of a new output file must already exist. Engine commands that still use
ambient filesystem paths are disabled for automation until they are migrated to the same
capability interface. Interactive desktop file pickers retain normal user-selected access.

## Headless server

`photocraft-cli serve` keeps one headless engine session (no window, no GPU) and answers the same
JSON-lines envelope on stdio, or on `127.0.0.1:<port>` with `--port <port>` (loopback only; each
authenticated connection shares the session). TCP uses the same first-frame `auth` exchange and
token options as desktop control. Stdio does not require this TCP handshake because access is
inherited from the process pipe. It is the fastest way for a script or agent to make many edits:
no MCP framing, no app start-up per command. Configure its file access with the same
`--automation-read-root` and `--automation-write-root` flags. Implementation:
`crates/automation/src/rpc.rs`.

| Method | Params |
|---|---|
| `engine.execute` | `{command, params?, wait?}`: any engine command (`wait: false` starts a long one as a background job: `{job, pending}`) |
| `jobs.list` / `jobs.cancel` | `{}` / `{job?}`: background jobs; cancel one or all. Every request (and MCP tool call) first applies the jobs that finished, so `doc.save`, `doc.inspect`, `doc.render` and `session.list` include a finished job's result without polling `jobs.list` first |
| `engine.commands` | `{filter?}`: registry with params docs and enablement (`enabled`, `disabledReason`, `needsDocument`); `result` is a bare array of command objects ([Command list replies](#command-list-replies)) |
| `session.list` | open documents and the active index |
| `doc.open` / `doc.new` | `{path, xcfProfilePolicy?, nativeProfilePolicy?}` / `file.new` params |
| `doc.place` | `{path, scale? (percent), fit? (true), center? ([x, y]), index?}`: places a file as a Smart Object layer above the active one and returns `{layer, scale, bounds}`. `path` resolves beneath the read root, so this is the capability-scoped `file.placeEmbedded` — that command itself still refuses an ambient path |
| `doc.exportLayers` | `{dir, format? (png), prefix?, visibleOnly? (true), quality?, index?}` → `{files, count}`: one file per exported layer, named like `file.export.layersToFiles` names them. `dir` resolves beneath the write root, so the ambient command stays refused. The bytes come from the command's own core, so both paths write the same files; the write capability refuses to overwrite an existing file |
| `doc.exportDataSets` | `{dir, format? (png), dataSets? (all), naming? ("{name}\|{index}\|{document}"), index?}` → `{files, count}`: the capability-scoped `file.export.dataSetsAsFiles`. The batch is planned first and colliding names are refused before any file is written, as in the command |
| `doc.importDataSets` | `{path, delimiter? (,), index?}` → `{imported}`: the capability-scoped `file.import.variableDataSets`; the CSV is read beneath the read root |
| `doc.save` | `{path?, format?, quality?, index?, tiffLayers?, tgaBits?}` (`.pcraft` native, else export by extension; set `tiffLayers: true` to preserve TIFF layers rather than exporting a flattened TIFF; `tgaBits` 24 or 32 sets a TGA's bits per pixel, as `app.save` does). Without `path` only a PSD, PSB or `.pcraft` document is written back to its own file, in its own format; anything else is an error and the file is left unchanged. A successful layered save (PSD, PSB or `.pcraft`) records the current revision as saved and makes the file the document's path, so `session.list` reports `dirty: false`; a flat export is a copy and leaves the document dirty |
| `doc.inspect` | `{index?}`: same JSON as `document.inspect` |
| `doc.render` | `{index?, maxSide? (1024; 0 = full), path?}`: PNG to `path`, else `{mime, base64}` |
| `doc.select` / `doc.close` | `{index}` / `{index?}` |
| `batch` | `{steps: [{command, params?, wait?} \| {method, params?}], stopOnError? (true)}` → `{completed, failed, results}` (a step with `wait: false` starts a long command as a background job, like `engine.execute`) |
| `methods` | the list above |

Explicit owned XCF Open accepts exactly `{"path":"source.xcf","xcfProfilePolicy":"keepCurrent"}`
through JSON-lines `doc.open` or MCP `doc_open`. The policy is optional; omitting it preserves
ordinary Open. These finite plain, Threshold Alpha and RGB nonlinear 8x6 Histogram
Threshold routes use the existing read authority and
catalog quota, retains current interpretation and returns an `owned-import-summary` with
warnings/assessment. Unknown, null or extra policy fields refuse before file I/O; stored
locators grant no access. MCP bridge mode forwards the same two fields to `app.open`.
The retained JSON-lines transport keeps its paid complete reply through writing; direct
raw-Value `Headless::handle`/`respond` and nested generic batches refuse this explicit policy.
The typed Headless owned method remains available to callers that retain its reply carrier.

For an active owned Threshold Alpha, Histogram Threshold or Point document, single-command
`engine.execute` can run `xcf.filter.inspect` with exactly
`{"filterId":"1"}` (replace `1` with the actual canonical nonzero decimal ID).
The reply preserves the existing exact argument, operation, owner and revision
fields; inspection supplies no activation or source-fidelity proof. CLI `run`
and MCP `command_run` support the same command. These routes inspect the installed
snapshot without applying queued jobs. Retained RPC and desktop control keep the
paid reply through writing and flushing; MCP uses its bounded SDK text handoff.
Bare-Value Headless methods and generic batches refuse accounted inspection;
typed callers use `inspect_owned_filter_bounded` and retain its returned carrier.
The same owned routes also accept `xcf.inspect` with null/omitted parameters or
`{"offset":0,"limit":64}`. Offset is at most 100000; limit is 1–128. Each metadata
collection is paginated independently. `filters.entries[].id` supplies the canonical
ID for subsequent `xcf.filter.inspect` or separately authorized filter edits.
Stack revisions remain decimal strings in project discovery. Typed callers use
`inspect_owned_command_bounded` with exactly one of these two command IDs and retain
the returned carrier. Resident inspection keeps its existing behavior. Inspection
of other owned classes remains separate work.

`xcf.inspect` also supports coherent flat plain documents held by the actual
accounted owner. It includes paginated resources, saved sets and independently
paginated set members. Resource locators remain inert and `accessGranted` stays
false; inspection reads no resource payloads. This describes the current model
and establishes no original-XCF lineage, profile interpretation, render support
or edit permission. Plain documents do not enter the retained filter-inspection
route. The same paid replies, transport boundaries and authorizers apply.

Those owned filter classes also accept current `layer.setProps` name,
visibility and opacity edits, for example
`{"layer":1,"name":"Foreground","visible":false,"opacity":0.5}`. The layer
target may be omitted or given as the existing numeric ID or exact name.
Combined fields publish one undo step; canonical names/visibility, opacity
casting and changed-visibility locks follow upstream behavior. Layer opacity
uses the existing Normal composition math; filter opacity is a separate field.
Other properties and coalescing remain refused for these classes; background
starts refuse before dispatch normalization. Source/native samples, masks and
profiles stay retained, and no new original-source eligibility follows.

The Point storage path now joins the existing 24 PointReplace family variants
(19 stored GEGL names), retaining each family's source, precision, geometry,
profile and composition restrictions. Those family paths retain singleton
qualification. The separately qualified witnessed Exposure/Invert stack uses
one or two nodes with its original order, composition and precision rules;
arbitrary mixtures of the singleton families remain unsupported. Ordinary
default Open enrollment and other stock families are separate work. Paid Point
inspection exposes bounded literal Color storage as exact words, independently
of evaluation or editing eligibility; resident inspection is unchanged.

The same holder also carries the separately qualified Desaturate, Saturation
RGB/Gray/YUV, Value Invert precision and original Posterize families. Their
distinct schema/composition epochs and original model, precision, encoding and
geometry requirements remain mandatory. Posterize uses exact signed width-32
integer arguments; other widths and integers outside the i32 range refuse.

Additional separately qualified scalar routes cover offset Exposure, HDR RGB
Clip, Unpremultiply, original and edited Mono Mixer, and original Value Invert.
Their original source/current geometry, precision, profile, finite-value and
mask restrictions remain. The opaque original Mono Mixer, Value Invert and
Posterize paths keep their legacy wide results until composition, without
adding an intermediate U8 rounding step. Unpremultiply has no stored arguments;
activation still uses `xcf.filter.enable` and requires operative validation.

For a qualified current Point document, existing `xcf.filter.set` and
`xcf.filter.enable` accept activation and up to ten unique existing arguments.
Arguments retain their exact Float32Bits, signed width-32, Bool, Enum or four-word Color shape;
literal colors require `nonlinear_profile` encoding and a null profile. Unknown
names, changed raw types or enum domains, unsupported structures and non-unit
filter opacity refuse. The current and proposed complete operative recipes must
both validate before one undo step is published. Exact no-ops preserve redo;
failed preparation leaves the installed snapshot unchanged. Native continuation
uses `.pcraft`; the original XCF stays read-only. These command paths are
synchronous and use the existing transport authorization and cumulative work
limit.

The same qualified Point path accepts the existing Color Balance, Levels and
FREE Curves configuration shapes. `xcf.filter.set` preserves their nested
Config field names and order, exact Float64 hexadecimal words and existing
Curve array shapes. FREE Curves retain five channels of 256 samples; aggregate
F64 input is capped at 1280 words before temporary conversion. Changes to the
stored shape, unknown fields and nonfinite operative numbers refuse. Paid
`xcf.filter.inspect` returns the complete configuration, including exact raw
words, under its own bounded reply owner. Original source/precision/profile
qualification and complete current/staged validation remain mandatory.

For the qualified witnessed Exposure/Invert stack, `xcf.filter.move` accepts
exactly `{"filterId":"123","index":0}` with an owner-local index
within the current stack. Stable IDs also target either node for inspection,
argument changes and activation. A changed move publishes one undo step;
moving to the current index preserves redo. The complete current and proposed
stack must validate, including its original saved-order interpretation and
half-opacity Invert rules. Content locks, source protection, synchronous
dispatch and the shared cumulative work limit still apply.

`xcf.filter.remove` accepts exactly `{"filterId":"123"}` for removal from a
qualified two-filter witnessed stack. It preserves the survivor, archives the
removed mask's source-property records with their exact raw bytes, and commits
one undo step. Required or dependent mask state and archive-name collisions
refuse atomically. Removing the last witnessed filter retains its empty stack,
owner and incremented revision. The closed current empty recipe preserves the
underlying pixels and Canonical output metadata; paid discovery reports zero
filters while preserving the stack row, and native `.pcraft` continuation keeps
that state. Filter-ID queries and edits then have no target. Empty continuation
retains the original RGB U8 nonlinear 8x6, implicit-profile and Normal-composition
restrictions. Insertion requires separate template support.

For explicit owned native reopen, use exactly `{"path":"current.pcraft","nativeProfilePolicy":"keepCurrent"}`
with `doc.open` or MCP `doc_open`. Exactly one profile policy is permitted; both together,
null/unknown values, extra explicit fields and mismatched suffixes refuse before job
synchronization or file access. With neither policy, legacy ignored extension fields and
ordinary Open remain unchanged. The native route retains the actual current interpretation
on the catalog quota and returns an `owned-native-summary`; persisted original-source
locators restore no read authority. The retained RPC and MCP handoff rules above also apply.


For an active owned document, Headless `doc.save` supports native `.pcraft`
Save/Save As through the same prepared engine transaction used by the desktop.
The installed trusted or write-root capability remains mandatory; source aliases
and fingerprints are checked before the atomic write. Nonactive owned targets
and flat exports refuse without switching tabs. CLI
`run source.xcf --xcf-profile-policy keepCurrent --cmd select.all --out copy.pcraft`
uses this route. Unsupported output suffixes or formats fail before opening the
input; saving produces a native project and does not write XCF.


```sh
printf '%s\n' \
  '{"id":1,"method":"doc.open","params":{"path":"in.jpg"}}' \
  '{"id":2,"method":"batch","params":{"steps":[{"command":"image.adjustments.invert"},{"command":"filter.blur.gaussianBlur","params":{"radius":3}}]}}' \
  '{"id":3,"method":"doc.save","params":{"path":"out.png"}}' | \
  photocraft-cli serve --automation-read-root /work/project --automation-write-root /work/project
```

The MCP server has the same batching as the `command_batch` tool (`{steps:[{id, params, wait?}], stop_on_error}`),
in headless and bridge mode. A step with `wait: false` returns `{job, pending}` at once, as `command_run`
does; later steps that edit the same document fail with a message naming the job until it ends.

## Transport limits

The desktop and headless TCP listeners currently enforce:

- a 1 MiB maximum encoded request line;
- an 8 MiB maximum encoded JSON reply, including the newline;
- at most 16 simultaneously serviced connections per listener;
- a 30-second socket read/write timeout;
- at most 256 steps in a headless `batch` or MCP `command_batch` request.

An oversized line, excess connection, unauthenticated request, or unauthorized filesystem path is
rejected before command dispatch or file effects. The headless JSON-lines stdio server also
enforces the request and reply byte ceilings; MCP over stdio (`photocraft-cli mcp`) does not
cap request bytes, because the MCP SDK reads its own request lines (only MCP tool results are
checked). A TCP connection is closed after an oversized
line; on stdio an oversized or non-UTF-8 line gets one error reply (`id: null`), the rest of that
line is skipped without being dispatched, and the session and its open documents keep serving. MCP tool results are checked as encoded JSON,
including the text/image content envelope, and the MCP bridge bounds incoming desktop replies.
A `batch` or `command_batch` stops at the first step whose result no longer fits the reply budget
(that step may have run; later ones do not). MCP charges each result at its size escaped inside
the text content, so the reply still lists `completed`, `failed` and every result so far, ending
with the budget error.

Headless automation previews (`doc.render` / MCP `doc_render_preview`) allow a maximum requested
edge of 2048 pixels and a source document of at most 67,108,864 pixels. `maxSide: 0` (MCP
`max_side: 0`) still means full size, but fails if the document's longest edge exceeds 2048.
PNG results are capped at 5 MiB before base64 encoding or writing a rendered file. Preview
dimension/source checks run before compositing; the encoded-PNG check runs after encoding.
Explicit trusted-local CLI rendering keeps its existing behavior.

Batch replies have an aggregate byte budget with space reserved for the outer reply and ID.
Exhausting it stops later steps even when `stopOnError` is false. A result can exceed the reply
budget after an edit has run: the error says the operation may have completed. Earlier steps
are not rolled back; inspect state before retrying. The MCP bridge drops an oversized incoming
reply without automatically retrying the operation. Bridged screenshots are decoded with
8192-pixel edge, 16,777,216-pixel, and 64 MiB allocation ceilings, even without downscaling.

These ceilings do not implement explicit JSON-depth policy, total session/document-memory
accounting, compositor scratch-space accounting, command cancellation/duration limits, or
general per-method capabilities. Desktop screenshot capture/encoding and document import/export
still need their own operation budgets; the desktop reply ceiling applies after the UI creates
its response. A bounded output does not imply bounded command cost.

### Rendering modes

`performance.renderingMode` accepts `auto`, `gpu`, or `cpu`. Automatic is the default for new
settings; older `useGpu: false` or `gpuBackend: cpu` preferences continue to select CPU mode
until an explicit mode is saved. Changes apply at the next launch. Automatic and GPU both
fall back on graphics errors rather than risk documents. CPU compatibility composites images
on the CPU and prefers software window adapters, when available. macOS still uses Metal for
the window. A failed window renderer initialization retries once in CPU compatibility mode;
a driver process crash is detected by the startup marker on the next launch.

### Camera Raw dialog

On the desktop the dialog is a window of its own (title bar, moved and resized like any window), as Adobe Camera Raw is; the main window is dimmed and takes no input while it is open, and closing the window is Cancel. Where the platform has no extra windows (the web build) it is drawn over the main window.

**Screenshots:** `ui.screenshot` captures the main window only, and eframe cannot capture an
immediate viewport, so while Camera Raw has its own window the PNG shows the dimmed main window,
not the dialog. Read the dialog's state through `ui.inspect` (`cameraRaw`: params, histogram,
zoom, control rectangles) and drive it with the `ui` params below. To *see* it, render offscreen
(`cargo run -p photocraft-ui-egui --example snapshot -- --script '[["ui.menu.invoke", {"id": "filter.cameraRaw"}]]'`
or an `egui_kittest` harness): those embed
viewports, so the dialog is drawn over the main window and appears in the image.

`ui.menu.invoke {"id":"filter.cameraRaw","params":{}}` opens Camera Raw on the active
RGB/Grayscale layer. `params: {"smartFilter": {"layer": id, "index": i}}` opens it on an existing
Camera Raw smart filter instead (as double-clicking the filter in the Layers panel does): the
stored settings over the pixels below that filter, previewed through the filter mask; commit
then runs `layer.smartFilter.setParams` with every setting. `params.ui` accepts `set` (filter settings), `before`, `scope`, `view`, `commit` and
`cancel`. All parts of one request are validated before any is applied; a rejected request
leaves the settings, view state, preferences and document unchanged (and closes a dialog it
opened). Unknown `ui` or settings properties, non-boolean `before` / `commit` / `cancel`, and
`commit` together with `cancel` are errors. Point curves (`pointCurve`, `pointCurveRed`,
`pointCurveGreen`, `pointCurveBlue`) are empty or 2–16 finite points in 0–255 with inputs at
least one level apart. Commit dispatches one `filter.cameraRaw` engine command; a failed commit
keeps the dialog open for correction. Nothing else writes document history.

**Opening a raw file.** An interactive open (File › Open, Open Recent, drag and drop, the
command line) of a camera raw developed from its sensor data shows this dialog first, titled
"Camera Raw (name)", with **Open** and **Cancel**, as Photoshop opens raws in Adobe Camera Raw.
`ui.inspect.cameraRaw.openingRaw` is then `{name, path}`. Commit (Open) re-develops the raw when
Temperature, Tint, Exposure, Shadows, Highlights, Clarity, Texture or Dehaze changed (as-shot
white-balance gains, develop exposure, and the other five in linear light before the camera's
tone curve, as Camera Raw does for raw files; without an as-shot white balance, or without the file's bytes, they stay RGB
adjustments), applies the remaining settings as one `filter.cameraRaw` step and leaves the
document unmodified; its result is `{document, redeveloped, filter?}`. Cancel closes the
document. `app.open` and other automation opens never show the dialog. The preference
`rawDefaults.openInCameraRaw` (default `true`) turns it off.

Imported PSD Camera Raw filters whose processing settings are all mapped or neutral use this
same editor. `params.__cameraRawPsd` is reserved import/export metadata: preserve it when editing
stored parameters. Other processing fields or versions remain opaque Photoshop filters. See
[PSD Smart Filters](camera-raw-histogram.md#psd-smart-filters) for the supported settings and export
limits.

The response and `ui.inspect.cameraRaw` contain:

- `histogram`: `source` (`before` | `after`), `size`, `approximate`, 256-bin `red` / `green` /
  `blue`, `samples`, `transparent`, `invalid`, per-channel `underflow` / `overflow`, and exact
  endpoint counters `shadows` (≤0) / `highlights` (≥1). Counts describe the bounded preview
  proxy in its RGB sample domain, not full-resolution or ICC display-gamut statistics.
- `previewRevision` (advances only when the proxy is re-developed), `renderMs`, `histogramMs`
  (CPU only; zero on WebAssembly).
- `curveState` (`selected`, `drag`) and `curveRect: [left, top, right, bottom] | null` (null while
  the Curve section is closed). Changing `set.pointCurve` cancels a stale gesture.
- `scope` (the view state below), `hasScopeSelection`, `vectorscope` (`bins`, `samples`,
  `before`, `selectedRegion`) or null, `scopeMs`, `scopeRevision`, `overlayRevision`.
- `pointerReadout` and `samplerReadouts`: `position`, `space` (`rgb` 0–255 in the preview sample
  space, or `lab` through the document ICC profile → D50), `values`, `alpha`; null on invisible
  pixels.
- `hoverSample`: `position`, `rgb` (histogram domain) and `hueSaturation` (turns, 0–1; only
  while the vectorscope is shown), or null.
- `hoveredZone`, `previewRect`, `scopeRect`, `vectorscopeRect`.
- `view` (`zoom: null` for fit, or a numeric factor where 1 = 100%, `center: [u,v]`,
  `hand`), `zoom` (effective factor, null before layout), `sourceSize`, `viewportRect`.
  At 100%, one source pixel maps to one physical display pixel, including on HiDPI displays.
- `previewApproximate`, `detailPending`, `detailError`: whether the visible image still uses
  the proxy, whether full-resolution refinement is running, and any refinement error.

`params.ui.view` changes navigation only: `zoom: "fit" | 0.0001..=16`, `center: [u,v]`
(finite coordinates in 0..=1, clamped to keep the image within view), `hand: bool`. Fit recentres
and follows window resizing; navigation resets when opening a different Camera Raw dialog.
Invalid mixed requests are atomic, just like scope and filter settings. View requests do not
re-develop the proxy or change filter parameters/history.

With the Zoom tool (Z), click toggles Fit / 100%; click-drag right/left scrubs in/out.
Ctrl-drag (Command on macOS) fits a rectangular region. H selects Hand; Space temporarily pans,
including while placing colour samplers. Alt/Option + wheel zooms at the pointer; an unmodified
wheel pans. Pinch zooms. Ctrl/Command +/- zoom, 0 fits, and Alt/Option + Ctrl/Command + 0 shows
100%; Ctrl/Command+Shift-click also selects 100%. Right-click opens Fit / 100% / preset
percentages. Tool double-clicks fit the image.
Text fields retain their keys and wheel gestures over the settings panel still scroll it.

The proxy remains the fast path while changing settings. Where more source pixels are needed,
Camera Raw lazily refines with the **same engine Camera Raw pipeline** at pixelScale=1 and the
same selection/filter-mask coverage. Native refinement runs off the UI thread (one revision at
a time); stale results are discarded. Only a bounded visible crop is uploaded to the GPU, and
pan/zoom reuse developed pixels. Before and neutral settings read the original pixels directly.
Absurd proxy domains (over 512 MP, including distant sparse off-canvas pixels) are rejected
before reading pixels. Full-resolution refinement is capped at 64 MP to bound the existing full-image engine pipeline;
above that it explicitly reports unavailability and keeps the proxy. Oversized viewport crops
also retain the proxy. Without a browser worker, wasm explicitly retains the filtered proxy
rather than block its UI; Before and neutral settings still crop the original source pixels.
The histogram and vectorscope continue to analyse the bounded proxy; readouts and clipping
warnings use the full-resolution pixels once refinement is visible.

`params.ui.scope` changes presentation only: `shadows` / `highlights` (clipping warnings, U / O),
`lab`, `samplerTool` (S), `vectorscope`, `selectedRegion` (requires a document selection),
`redRight`, `hideSkinLine`, `floating`, `floatingRect: [left, top, right, bottom]` (200×150 to
4096×4096 points), `sample: [u,v] | null`, and the probe list: `samplers: [[u,v], …]` replaces it,
or `clearSamplers: true`, `removeSampler: index` and `addSampler: [u,v]` edit it, applied in that
order (the two forms can't be combined). Coordinates are normalized to the displayed proxy; at
most nine probes. `tone: {zone: "blacks|shadows|exposure|highlights|whites", delta}` adjusts the
same parameter as a histogram drag (`reset: true` sets it to 0); Exposure clamps to ±5 stops, the
others to ±100.

Hover, probe movement, theme changes and panel resizing never re-develop the image; Before/After
and region changes rebuild only the dependent analysis. Display options and the floating panel
geometry persist in preferences `dialogs["filter.cameraRaw.scope"]`; probes and vectorscope
visibility reset when the dialog opens. HDR scopes are not implemented. See
[camera-raw-histogram.md](camera-raw-histogram.md).

### Editing and composing actions

Creating an action scrolls its row into view; newly recorded steps are revealed immediately.
Drag a step above or below another step in the same action to reorder it. Drag an action header
to reorder the action list. The insertion line marks the drop position; dragging near the rows'
top or bottom scrolls the list. Reordering is saved with the actions and does not undo image edits.

While recording, press a function key assigned to another action (or play it from the panel).
The recorder adds **Play Action: <name>** as one step, and the called action still executes.
Playback resolves the name each time, so editing or moving the called action does not freeze or
retarget the caller. Reassigning F6 later does not change an already recorded named call.
Missing actions, recursive calls and nesting beyond 16 levels fail visibly; a child failure stops
the parent before its next step. Every nested command retains the normal automation permission check.
Existing recordings containing expanded commands remain unchanged; remove those steps and record
the named call again if desired.

### Qualified XCF linked geometry

`xcf.link.setPlacement` takes `layerId` as a canonical decimal U64 string and
`originPx: [x, y]` as bounded integers. `xcf.link.setTransform` takes the same
identity, `matrixF64Bits` as nine lower-case sixteen-digit hexadecimal strings,
`interpolation: "none|linear|cubic|nohalo"`, and optional `originPx`. The matrix
uses the original linked source's pre-offset. Query command metadata for the
enabled state; these commands require a qualified resident linked image with
explicitly recovered owned PNG material and unlocked content/position.

Successful changes regenerate current pixels, update current bounds and support
undo/redo and native save/reopen. Original source transform/cache/provenance and
resource locators remain intact; the commands do not read a locator or grant
resource access. Unsupported sampler/geometry/material contexts return an error
before publication. `xcf.inspect` and changed-command responses expose current
matrix bits in the same string form, preserving all 64 bits across JSON clients.


## Pending import recovery

The GIMP Filters panel exposes stacks in pages of 64 rows. `ui.set` accepts
`xcfFilterStackPage` with exactly `document`, `layer`, `revision`, and `offset`;
identities accept exact U64 values or canonical decimal strings, and `offset`
must be a multiple of 64 from 0 through 960. `null` clears the cursor.
`ui.inspect` returns its identities as decimal strings. A restored or stale page
resets when the panel resolves the current stack; row actions still capture and
validate their own current document, filter identity and revision.

Matching Named Layer item sets have an independent `xcfItemSetPage` cursor with
the same identity fields. Its offset ranges from 0 through 9984 in multiples of
64 and counts matching sets in their original order. `ui.inspect` returns exact
decimal-string identities; stale/restored pages reset without granting an edit.

Source Details uses `xcfSourcePageDraft`: `document` and `revision` are canonical
decimal strings, `owner` is `{kind: "layer|channel|mask|path", id: "…"}`, and
`offset` is a multiple of 32 from 0 through 100000. Legacy layer cursors may use
`layer` instead of `owner`; a supplied legacy layer must match the typed owner.
Resource Details uses `xcfResourcePageDraft` with exactly `document`, `revision`,
`offset` (multiples of 64 from 0 through 960), and boolean `open`. These identities
also use canonical decimal strings. Both fields accept `null` to clear the view;
`ui.inspect` returns their lossless wire forms. Navigation alone performs no query
or recovery. Resource reports require explicit Inspect, and stale targets reset
their cached reports. Filter insertion choices use the same 64-row stack pages.

Composition controls expose `xcfCompositionDraft` with exactly `document`,
`revision`, `layer`, `sourceMode`, `blendSpace`, `compositeSpace`, and `operator`.
Identities are canonical decimal strings. Spaces are `null` or 1 (linear RGB),
2 (profile RGB), 4 (perceptual RGB); operators are `null` or 1 through 4 (union,
clip to backdrop, clip to layer, intersection). Setting the draft clears its
private edit origin and performs no document operation. `null` clears the draft.
Agents can dispatch `xcf.composition.set` directly with explicit supported
policies and paired `documentId`/`expectedRevision` strings; stale pairs refuse
before floating-preview preparation. Retained Auto resolution is read-only.

Pending decoded imports that could not enter the profile-question queue can be
released with the registered engine command `file.open.discardPending`.
It accepts an empty object to discard all private pending imports, or
`{"heldId":"7"}` to discard one canonical nonzero decimal-string identity.
The result contains `discarded` and, for a single import, `heldId`. Unknown
identities and malformed fields fail before removal. The command uses the normal
authorization gate, needs no filesystem grant of its own, and preserves open
tabs, document views and Undo/Redo. It does not clear the shell's visible profile
question queue; the UI's Cancel action coordinates that queue separately.

## Atomic XCF linked-image substitution

`xcf.link.substitute` is a registered engine command for an explicit Reduced
replacement. Its params contain exactly six strings: `documentId`,
`expectedRevision`, `layerId`, `resourceId`, `policy: "substitute"` and
`payloadHex`. IDs are nonzero canonical decimal U64 strings; the revision is a
canonical decimal U64 string that may be zero. No JSON numbers, leading zeros,
extra keys or `coalesce` key are accepted. PNG input is capped at 8 MiB (16 MiB
hex characters), but transport limits still apply: the encoded JSON request line
remains capped at 1 MiB. Large replacement bytes should use the path interface.

`xcf.link.substituteFromRoot` takes the same five identity/policy strings with
`path` instead of `payloadHex`. The path is an explicit forward-slash relative
request under the server/application's held `--automation-read-root` capability,
not a source-stored locator or a client's filesystem grant. Absolute/traversing
paths, unavailable roots and unsupported material fail closed. The reader caps
input at 8 MiB before constructing internal hex; that hex does not cross the
transport, so the 1 MiB request ceiling is unchanged.

```json
{"id":20,"method":"xcf.link.substituteFromRoot","params":{"documentId":"1","expectedRevision":"0","layerId":"2","resourceId":"3","path":"replacements/owned.png","policy":"substitute"}}
```

Use `documentId` and `revisionString` from inspection verbatim (`expectedRevision`
is `revisionString`); never convert the compatible numeric `revision` field.
Use inspected layer/resource IDs, not the illustrative values above.
The target is checked before reading and again at commit. The command is
synchronous, does not return a job, and requires no job polling. These new routes
refuse pending/running jobs rather than synchronizing them before the target
check; `engine.execute` with `wait: false` is refused for this command. Live UI
control may defer its reply while the bounded root read completes; that is not
an engine background job or a remote picker lease.

The MCP tool is `xcf_link_substitute_from_root`, with the same six camelCase
string fields. Headless MCP uses its own held root; bridge MCP forwards the
small path request to the app and never reads a client-side path. Existing
`command_run` can invoke the raw engine command for small hex payloads. Substitution
updates retained material and the qualified current raster in one undoable edit,
returning `byteIdentity: "Substituted"` and `assessment: "Reduced"`; it does not
recover original byte identity or authorize Exact/font recovery. XCF is import
only: save/reopen as `.pcraft`, using the configured write/read roots. Runtime
grants are not serialized by a native save.

The engine lane has scoped runtime evidence. The new small-path Headless/RPC,
MCP and live-control joins are implementation-first work: validation and the
remaining native callback joins are in progress. Their presence is not yet
end-to-end transport or physical picker qualification. Discover the running
methods/tools before use; a build lacking a route reports it unavailable.


## Exact linked-image recovery

`xcf.link.recover` uses exactly the six string params of the atomic replacement
command, with `policy: "exact"` and `payloadHex`. The linked resource must already
retain its original content hash, and the authorized PNG must match that hash.
Names, locators, cached renders or current pixels cannot supply a missing original
identity. The same 8 MiB carrier cap, canonical IDs/revision, current-target checks
and one synchronous undoable transaction apply.

For root-bound input, use `xcf.link.recoverFromRoot` or MCP
`xcf_link_recover_from_root` with exactly `documentId`, `expectedRevision`,
`layerId`, `resourceId`, `path`, `policy` as strings and `policy: "exact"`.
`path` is an explicitly requested relative file beneath the held read-root
capability. Unavailable authority fails closed; no retained locator is read
implicitly. The path request stays small; bounded internal hex does not cross
the unchanged 1 MiB transport frame. There is no job to poll. Live control can
await the bounded read before its final reply.

```json
{"id":21,"method":"xcf.link.recoverFromRoot","params":{"documentId":"1","expectedRevision":"0","layerId":"2","resourceId":"3","path":"originals/linked.png","policy":"exact"}}
```

Use inspected current identities/revision. Success returns `byteIdentity:
"Exact"` and `assessmentCommand: "xcf.assess"`; this describes the recovered
linked bytes, not whole-document Full fidelity. Query `xcf.assess` for current
dependencies and remaining limitations. Save/reopen as `.pcraft` under explicit
write/read roots; runtime grants do not survive native serialization. Exact
panel/control/root/MCP joins are implementation-first work with validation in
progress; discover the running interfaces rather than infer availability from
this contract. Complete XCF/US3 and physical picker acceptance remain separate.


## Current imported editing state

Document inspection now includes `xcfEditingState` for retained XCF state
(`null` for documents without it). It exposes semantic selected items/paths,
active item and mask target, runtime active/selected layers, saved item-set
members and inert rule text, floating attachment, mask owner/bounds/offset/flags,
and filter-mask identity/bounds. Item references are `{kind, id}` with canonical
decimal string IDs; set IDs and floating-layer/filter IDs are strings too.

Each page has `total`, `offset: 0`, `limit: 64`, `entries` and `truncated`.
Names/rule expressions are capped at 256 characters with truncation flags.
Mask discovery scans at most 10,000 item records: when that scan is truncated,
`total` is null and `matchingInScan` is only the observed lower bound. The
existing paginated `xcf.inspect` command remains available for further metadata.
`xcf.source.inspect` accepts `{owner:{kind,id},offset?,limit?}` for exact typed
text/vector/linked source configuration. Owner IDs are canonical nonzero decimal
U64 strings; pages contain at most 64 entries. Integer values use decimal strings
and F32/F64 values use fixed-width hexadecimal bits, preserving signed zero and
NaN payloads. Replies are bounded to 64 KiB, depth 8 and 1,024 typed nodes;
oversized nested values return an error. This query does not read cache payloads,
resolve locators, grant resources or qualify rendering.
Rules are reported, not evaluated; semantic selections and runtime focus are
shown separately. No resource bytes, locator lookup, permissions or grants are
exposed or acquired by this view. Use exact current typed references for supported
edits rather than infer identities from names, row positions or source tattoos.

The exact/substitute root routes still use explicit held authority and stale
revision checks. The view does not imply every inspected set/floating/mask edit
has a command, or whole-document Full/native-runtime qualification. Two-axis
text layout/caret/render callers are being migrated to checked physical X/Y
resolution; unequal-DPI editing and PSD/text-cache fidelity remain implementation
work until their acceptance results are available. No scalar fallback or testing
claim follows from the new inspection fields.

`image.imageSize` accepts independent `resolutionX` and `resolutionY` values in
pixels per inch, each finite and in `(0, 10000]`. Omitted axes and the current
physical unit are preserved. The existing scalar `resolution` sets both axes;
combining it with either axis is an error before mutation. With `resample: "none"`,
these edits change physical resolution while preserving pixel dimensions and
samples. Undo/redo restores the complete pair and unit. Acceptance tests for this
new interface remain deferred during the implementation pass.

For Image Size dimensions, `sizeUnit` is `px` (default), `in`, `mm`, `pt`, `pc`,
or `document` (the retained unit, including a custom unit). Width and height
use their respective DPI axes, including explicit resolution changes in the same
request. An omitted dimension keeps the pixel aspect ratio. Physical dimensions
round to the nearest pixel and refuse values outside 1–300,000 pixels; they are
never silently clipped. Unit selection applies to this request and preserves the
document's retained unit definition.

`xcf.item.locks.set` takes a canonical decimal-string `layerId` and at least one
explicit boolean among `content`, `position`, `alpha`, and `visibility`. It edits
that imported layer's current flags in one undo step; native pixel, position and
transparency flags track the corresponding explicit changes. Parent locks, native
Lock All and original source flags remain intact. It refuses transient previews,
background dispatch and coalescing.

`xcf.items.select` replaces persisted semantic selection with exactly four fields:
`selected` (typed references), `active` (typed reference or null), `selectedPaths`
(path references), and `maskTarget` (mask reference or null). References use
`{kind, id}` with canonical decimal-string IDs. It validates current membership,
duplicates and active/selected coherence and creates one undo step. Runtime
layer/channel/path focus and resource authority remain unchanged. These new
commands have drafted regression tests; acceptance execution is deferred.

`xcf.itemSet.create` takes exactly `setId` (an unused canonical decimal-string ID),
`name`, `family` (`layer`, `channel`, or `path`) and ordered `members` (`{kind,id}`
references). It creates a Named set with at most 1,000 unique current members of
that family. Existing set IDs are refused; no search rule is evaluated. Creation,
rename, deletion and membership edits refuse transient pixel previews before any
dispatcher normalization. Creation adds one undo step and preserves existing sets
and source metadata. Its acceptance tests remain deferred.

`xcf.mask.enabled.set` takes exactly `maskId` (canonical decimal string) and
`enabled` (boolean). The current implementation handles an admitted canonical
layer mask, synchronizes native/current enable flags in one undo step and keeps
its samples, extent, defaults and source data intact. Filter/channel masks and
unqualified mask pipelines refuse. Native Lock All and inherited imported content
locks apply; native Pixels-only locking permits this mask operation. Transient
previews, coalescing and background execution refuse before mutation. Validation
of this new command remains deferred.

The existing `layer.layerMask.enabled` menu command uses this checked path for a
canonical imported layer mask and preserves its `{enabled: boolean}` reply.
Ordinary non-XCF mask toggles and mask-link toggles retain their existing routes.

`ui.set` accepts `xcfItemSetDraft` as a whole object with exactly five fields:
`document`, `layer`, `revision`, `setId`, and `name`. Identity fields use canonical
decimal strings (nonzero except revision); legacy u64 JSON integers remain
accepted. Copy `revisionString` without converting it through a floating-point
number. The replacement name is limited to 256 characters and 1,024 UTF-8 bytes,
without NUL. `null` clears the draft; omitting it leaves the draft unchanged.
`ui.inspect` returns its identities and revision as exact decimal strings.

This is view state only: setting a draft grants no authority, retargets no
document, and applies no engine edit. The saved-set panel shows at most 64 sets
containing the current layer and does not evaluate saved searches. Its explicit
actions use guarded engine commands; the new UI interfaces remain implementation
work pending validation.


## Precision pixel inspection

Use `engine.execute` with `command: "document.pixel.inspect"` and params
`{x?: i32 = 0, y?: i32 = 0, layerId?: canonical nonzero U64 string,
view?: "composite" | "native" = "composite"}`. Unknown fields, wrong coordinate
types, malformed explicit IDs and half-open pixel-coordinate overflow refuse.
The existing `document.pixel` f32 array remains compatible.

Composite view uses the currently qualified f64 compositor and returns finite
`compositeRgbaF64` plus exact `compositeFloat64Bits` strings; it never falls back
to f32. Unsupported domains, nonfinite composite output or bounded work exhaustion
error. An optional explicit `layerId` also returns `nativeSample`: current
resident raster `encodedHex`, model/sample type, channel count, sample encoding
and native byte order, before composition. Text/shape caches and backed material
are not raw raster targets. Sparse/off-canvas coordinates deliberately read that
Surface's stored default when no tile exists.

Explicit `view: "native"` requires `layerId` and skips the compositor, so stored
F64 nonfinite/negative-zero bits can be inspected as bytes without false JSON
numeric exactness. `compositeRequested: false` distinguishes this choice from
fallback. Replies include `documentId` and `revisionString`; the query adds no
history and preserves transient floating state. Only one output pixel is requested; source qualification/render work has explicit
limits. This is not a display oracle, whole-document Full fidelity or completed
XCF acceptance. Implementation validation remains pending. UI Info's
legacy f32 readout is a separate consumer and has not been upgraded by this query.
`xcf.items.visibility.set` takes `items` (1–1000 unique typed references with
`kind: "layer" | "channel"` and canonical decimal-string `id`) and `visible`
(boolean). Layer visibility updates the native layer and current imported state;
channel visibility updates saved semantic state. Runtime channel-view choices
remain separate. The entire batch honors inherited visibility locks and commits
as one undo step.

`xcf.channel.overlay.set` takes `channelId`, `colorF32Bits` (three lowercase
eight-digit hexadecimal strings), and `opacityF32Bits` (one such string).
Values must encode finite normalized f32 display metadata. It edits qualified
saved-channel overlay metadata without altering channel samples or original
source records. Spot and selection channels are currently refused.

`view.newGuide` and `view.moveGuide` accept optional `unit` (`px`, `in`, `mm`,
`pt`, `pc`, or `document`; default `px`). A vertical guide uses X DPI and a
horizontal guide uses Y DPI. Conversion uses the retained physical unit;
the current native guide model stores the resulting finite position as f32.
An optional `documentId` / `expectedRevision` pair uses canonical decimal strings;
the New Guide dialog captures it and refuses a changed target before editing.

`xcf.layer.affine.apply` edits a qualified resident U32/F64 RGB/Gray raster in
one undo step. Its exact envelope is `documentId`, `expectedRevision`, `layerId`
(canonical decimal strings), `scaleF64Bits` and `offsetF64Bits` (lowercase
16-digit hexadecimal), `domain` (`unit` or `hdr`), `overflow` (`clamp` or
`reject`), `rounding` (`exact` or `nearest`), `identity` (`finite-only` or
`preserve-raw`), and `roi` (`{x0,y0,x1,y1}` integer endpoints). It requires an
explicit current encoding/profile, preserves raw alpha/outside pixels, and
refuses editable text/vector/linked source carriers. It does not admit a new
serialized GIMP evaluator. Validation execution remains deferred.

`path.list` and `path.info` add `pathIdString` alongside the legacy numeric
`pathId`, allowing clients to retain full U64 identities without float conversion.
Bound-path edits check native/imported owner ancestry before transient preview
normalization and during edit validation. Source records retain their original
identities; native save/reopen uses `.pcraft`.
`path.set` admits at most 10,000 subpaths and 100,000 total knots before parameter
cloning. Points use exact `[x,y]` or `{x,y}` shapes and finite coordinates with
magnitude at most 1,000,000; malformed explicitly supplied handles refuse.

`xcf.indexed.pixel.set` takes exactly `documentId`, `expectedRevision`, `layerId`
(canonical decimal strings), `x`/`y` (i32), and `index`/`alpha` (u8 integers).
It changes a current indexed ordinal and independent alpha in one undo step,
preserving duplicate palette entries and original `IndexedSource`. The current
scope requires an ordinary resident Indexed/U8 RGB carrier with coherent declared
bounds and palette; source-item/filter/effect carriers refuse. Content and
effective alpha locks, cancellation and staging limits apply.

`document.pixel.inspect` additionally returns `currentIndexed` for an explicit
current indexed layer: exact ordinal, independent alpha, effective transparent
index alpha and palette RGB bytes. Offbounds returns no invented ordinal default.
Native carrier bytes remain separate in `nativeSample`.

`xcf.filter.insert` requires `layerId`, an unused `filterId`, same-owner
`templateFilterId` (canonical decimal strings), `index`, `operation`, `arguments`,
and `enabled`. It inherits the explicit current envelope of an unmasked/no-aux
qualified scalar template and requires the same exact operation identity and
argument types. Arguments use bounded typed Number/Bool/String/Enum/Color
records; integer and float-bit payloads accept canonical decimal strings without
f64 normalization. The joint wire budget is 16 KiB, 1,024 value nodes and depth 8.
Insertion validates operative rendering even when initially disabled and commits
as one undo step. Novel schemas and nested/resource argument families refuse;
source provenance and caches remain intact. Acceptance execution is deferred.


`xcf.text.name.set` changes a current imported text layer's literal name and
`customName` flag together. Supply exactly `documentId`, `expectedRevision`,
`layerId` as canonical decimal strings, `name` as nonempty UTF-8 (at most 4096
bytes, no NUL), and `customName` as a Boolean. The command preserves the text
configuration, source cache and original provenance. Changing the flag never
infers an automatic name. Content locks and captured-target checks apply.

`xcf.vector.fill.color.set` and `xcf.vector.stroke.color.set` change the current
opaque literal color of a renderer-qualified regenerated vector. Supply exactly
`layerId` and `pathId` as canonical nonzero decimal strings and `rgbaF32Bits` as
four lowercase eight-digit hexadecimal strings. RGB must be finite and in
[0,1]; alpha must be exactly one. The current vector configuration, geometry,
source bindings, working interpretation and renderer remain authoritative.
These commands run synchronously, refuse floating selections and background
jobs, and preserve inactive style fields and immutable source provenance.

`xcf.filter.graph.inspectPlan` inspects the retained structure of one Graph
argument without evaluating it. Supply a canonical `filterId` string, exact
`argument` name (at most 128 bytes), optional `maxDepth` (1..4096, default 32),
`offset` (0..16384, default 0), and `limit` (1..64, default 64). Pagination applies
independently to declaration nodes, declaration edges and topological nodes;
returned indexes remain absolute. Stored ports and version identities are
reported unchanged. Structural validity does not qualify operation, argument,
port or rendering semantics. Inspection preserves the installed snapshot and
does not publish queued edits. Implementation acceptance remains deferred.
`xcf.path.name.set` changes a current imported saved path's name and explicit
custom-name flag. Supply exactly `documentId`, `expectedRevision`, `pathId` as
canonical decimal strings, bounded nonempty `name` (4096 UTF-8 bytes, no NUL),
and Boolean `customName`. Reserved work-path names, conflicts and ambiguous
clipping-name associations are refused. Geometry, PSD raw records and immutable
source data remain exact; current clipping-path name associations are updated.

`xcf.filter.mask.enabled.set` accepts exactly canonical `maskId` string and
Boolean `enabled`. It currently supports only the already qualified native
all-zero empty-selection masks in three closed operation contexts. Disabling a
mask still validates its native samples and metadata; nonzero coverage remains
unsupported. The command checks operative ownership, inherited content locks,
renderer qualification, cancellation and snapshot/revision before one undoable
edit. This limited transition does not qualify arbitrary filter masks.

Filter SET/ENABLE, MOVE/REMOVE, template insertion and mask edits require
synchronous dispatch without floating attachments, background jobs or
coalescing. Property editors show rejected-command errors and verify their
captured snapshot before applying an edit. The Add menu duplicates a selected
existing qualified scalar template; it preserves exact numeric wire values and
uses a new bounded current filter identity. Tests and visual acceptance for
these implementation additions remain deferred.
`xcf.indexed.palette.set` accepts exactly `documentId`, `expectedRevision`,
`layerId` as canonical decimal strings, integer `slot` (0..255), and `rgb` as
three integer bytes. The palette is document-wide: every current indexed owner
is validated and expanded privately before one transaction. Pixel ordinals,
independent alpha planes, palette order/duplicate entries, transparent ordinal
and immutable source palettes are preserved. There is no per-slot alpha field;
this command edits RGB only. All affected owner content locks apply, including
for carrier qualification of an otherwise equal request.

Read-only XCF inspection and precision-pixel queries do not run edit bookkeeping
or repair unrelated transient views. Background assessment completion also
avoids edit bookkeeping; its result is tied to the original snapshot. Tests for
these implementation changes remain unrun.
`xcf.channel.name.set` accepts exactly canonical string `documentId`,
`expectedRevision`, `channelId`, nonempty UTF-8 `name` (4096 bytes, no NUL), and
Boolean `customName`. Native saved-channel and canonical item names/flags change
together; samples, overlay, selection/focus and original source flags are
preserved. The synthetic selection channel is excluded. Apply/Reset controls
in the Channels panel use a serializable draft and one guarded application.

The UI `file.saveAs` route additionally accepts `format:"pcraft"` with optional
string `path`. This explicit native route suggests `.pcraft` even if the current
writable destination is PSD, rejects other suffixes before encoding/writing,
and checks the captured document/revision/snapshot when the picker completes.
Original-source protection still applies. Without `format`, ordinary Save As
keeps its existing destination policy. The imported-project preservation control
uses the explicit native route; it never writes XCF.
`xcf.channel.locks.set` accepts canonical `documentId`, `expectedRevision`,
`channelId` strings plus at least one Boolean `content` or `visibility`.
It changes only the saved channel's own current lock flags. Inherited locks
remain effective; runtime channel visibility/target state is preserved.
Position/alpha locks are unsupported by this command.

`xcf.channel.indicates.set` accepts those three captured-target strings plus
`indicates:"masked"|"selected"`. It changes the current alpha-channel overlay
interpretation through the existing native consumer, preserving samples and
color/opacity. Spot and synthetic selection channels are excluded.

`xcf.mask.geometry.set` accepts canonical `documentId`, `expectedRevision`,
`maskId` strings, `crop:[x0,y0,x1,y1]` integer coordinates wholly inside the
current declared mask bounds, and `offset:[x,y]` integer placement. It copies
same-depth native mask samples exactly into checked output bounds, synchronizes
canonical bounds/offset metadata, and preserves native default bits. The current
closed lane requires a layer-owned, unlinked mask with density one, feather zero
and finite unit samples; filter/channel mask geometry is unsupported. Content
and position locks, cancellation, working/storage limits and full current
renderer qualification apply before one undoable installation.
The Paths panel carries the current saved `PathId` separately from legacy name
selection. Naming edits use that typed target and an Apply/Reset draft, and
synchronize the legacy display selection only after a successful edit of that
same path. Work/layer/empty selections clear the typed saved target. Derived
row preparation is bounded and shared per document snapshot/revision; stale or
oversized row data produces an error rather than a cloning fallback. This
implementation has not yet undergone UI or performance acceptance.
Retained source raster caches and owned resource payloads use shared immutable
byte buffers in memory. Document snapshots and metadata/history clones share
those buffers; native readers reuse authenticated cache blobs. This ownership
change preserves the existing serialized byte arrays and native descriptors,
and does not restore resource grants or change source protection. Existing
logical expansion/work/resident limits remain conservative. Performance and
native round-trip validation for the change are still pending.
Native single-file `.pcraft` saves use a prepared bounded archive stream in
headless, interactive desktop and desktop-control routes. Preparation occurs
before private temporary output is created; protected-source and final rename
checks still apply. Desktop control uses a separate held-root capability and
cannot fall back to the interactive writer. Browser downloads retain the
existing bounded byte-output route. All open documents contribute inert source
exclusions to interactive export validation and final writes. This implementation
does not enable the native_large profile or claim large-file acceptance.
Saved item-set rename/delete/membership commands accept the compatible existing
set selector plus an optional captured pair: canonical decimal-string
`documentId` and `expectedRevision`. Supply both or neither; supplied identities
must match before mutation. UI controls supply this pair and retain a live
snapshot witness. These operations refuse conflicting previews/jobs, bound
metadata/COW preparation and preserve unrelated selection and source records.
Rename/delete preserve existing rule metadata behavior; explicit membership
editing remains limited to Named sets.

Linked-image properties expose Refresh, explicit current Position Apply/Reset
and interpolation for an existing retained transform. The current matrix bits
are preserved. All actions use registered owned-material commands; they neither
read stored locators nor restore access grants. UI and headless synchronous
routing precedes queued-job publication for these guarded operations.
Native desktop, trusted-local headless and authorized Workspace opens now use a
bounded seekable archive candidate for `.pcraft`. XCF and wrapper content takes
precedence over misleading native suffixes. The same held file handle is reused
for any non-native byte fallback; Workspace opens retain their held-root policy.
Background open carries cancellation through decoding, private color-policy
processing, source selection/view preparation and final document publication.
Directory native loading uses the same controlled manifest/object decoder.
These routes keep the standard admission ceilings and do not enable native_large.

Retained graph arguments have explicit Inspect/Next/Previous property controls.
They call `xcf.filter.graph.inspectPlan` on demand, fence the captured snapshot
before and after the query, and display at most 64 node/edge metadata rows. The
page cursor is view state; it supplies no authoring, evaluator or resource authority.
`layer.renameLayer` additionally accepts lossless canonical `layerId` decimal
strings. Existing numeric `layer` and active-layer fallback remain supported;
passing both selectors is an error. Guarded in-place rename uses `layerId` plus
captured `documentId`/`expectedRevision`, preventing transport rounding above
2^53 while preserving existing trim and keyboard/menu behavior.

Apply Image and Calculations validate supplied numeric and selector types before
floating-state normalization or source preparation. They execute synchronously
through UI/control routing; a nonwaiting control request receives the completed
inline result. Controlled source/mask preparation rejects unsupported wide legacy
arithmetic before converting to F32, and failed preparation leaves the target intact.

`image.calculations` has an explicit native mathematical coverage mode:

```json
{
  "nativeCoverage": true,
  "coverageContext": "unit",
  "documentId": "7",
  "expectedRevision": "12",
  "source1": {"channelId": "101"},
  "source2": {"channelId": "102"},
  "blending": "multiply",
  "result": "newChannel",
  "name": "Product"
}
```

Use the current document, revision and saved-channel identities; the numbers
above are illustrative. Both sources must be same-depth Gray channels without
alpha or spot ink in that captured document. This mode supports multiply,
darken, lighten and subtract with source2 as the base, opacity 100, scale 1,
offset 0 and no mask or inversion. Integer multiplication uses nearest-half-up
native arithmetic; F64 remains F64. Darken/lighten preserve the base native word
on equal samples. It validates unit coverage, exact current
geometry and imported rendering context, then creates one native channel as an
undoable transaction. This is a caller-selected mathematical edit and does not
qualify a stored GIMP operation or expand supported source contexts. Existing
legacy/default parameter behavior remains available when nativeCoverage is absent.

Sizing dialogs capture their originating document, revision and snapshot in
runtime-only state. Confirm emits canonical documentId/expectedRevision pairs;
a changed document, stale revision or restored dialog without its runtime witness
refuses Apply. Dialog restoration does not provide editing authority.

Canonical current Indexed U8 images support integer Canvas Size translation and
nonempty Crop through their index/alpha planes. Duplicate palette entries retain
their distinct ordinals; original XCF indexed snapshots remain provenance. An
empty indexed owner, an indexed background extension needing new color entries,
or an unqualified imported composition/filter context returns a named error.
These paths require agreement between current planes, palette, bounds and RGBA8
carriers; they do not reconstruct ordinals from RGB colors.

The existing `image.mode.bits8`, `image.mode.bits16` and `image.mode.bits32`
commands accept optional paired canonical-string `documentId` and
`expectedRevision` selectors. Conversion prepares private native surfaces with
cancellation and budget checks and commits one undo step. Same-depth canonical
requests preserve redo. Indexed conversion requires a separately qualified
ordinal policy and returns an error for a depth change.

`image.trim` and `image.duplicate` also accept optional paired canonical-string
document/revision selectors. Trim uses bounded rendered tiles for ordinary color
comparisons and raw alpha occupancy for qualified plain wide rasters. Color-based
indexed Trim and composited wide Trim return named unsupported errors. Duplicate
prepares a separate document and remaps its references without changing source
history. Its merged mode supports ordinary eligible depths; wide, indexed and
unqualified imported merged contexts refuse before pixel conversion.

The existing RGB/Gray/CMYK/Lab mode and `edit.convertToProfile` commands now
prepare CMS conversions privately with bounded work and cancellation. Assign
Profile changes metadata without converting pixel bytes. Both profile commands
accept the same optional paired document/revision selectors; their dialogs
capture a runtime origin and reject stale or restored Apply requests. Existing
profile resolution and imported analytic-profile restrictions still apply.
Multichannel uses its separate ordinary-depth plate-conversion path, with
unsupported wide and imported contexts refused before F32 reads.

`image.mode.indexedColor` preserves legacy conversion by default. The explicit
`canonicalIndexed:true` option for ordinary RGB inputs retains the quantizer's
actual indices, palette order and quantized alpha as editable current planes.
Its RGBA8 carrier uses palette RGB even at alpha zero; this is a deliberate lossy
conversion policy, rather than preservation of the old hidden RGB carrier.
The dialog's “Keep editable palette indices” option defaults off. Imported XCF
indexed planes continue to use their separate exact import/edit policy.
Indexed Color, Bitmap and arbitrary rotation accept optional paired canonical
document/revision selectors and prepare bounded, cancellable private output.

Channel Duplicate (`channel.duplicate`) accepts a saved-channel selector or the
existing composite/color/Quick Mask selectors, `name`, `invert`, and `document`
(an open-tab index or `"new"`). Optional canonical decimal-string `documentId`
and `expectedRevision` must be supplied together and identify the active source.
The implementation captures both source and destination snapshots, prepares a
private result, and commits to an existing destination without switching the
viewed tab. The returned `document` index identifies the edited destination.
Same-format saved native Gray channels retain their raw samples. Qualified
finite-unit U32/F64 inversion uses native arithmetic; wide depth changes and
unqualified interpreted wide sources fail before float32 extraction. Imported
channel metadata duplication is limited to the same document and retains global
source provenance. New-image publication creates a grayscale document and uses
no source undo step.

Quick Mask (`select.editInQuickMaskMode`) accepts optional `on` and the same
captured target pair. Entry and exit prepare privately with cancellation and a
final snapshot/revision fence. Same-depth native U32/F64 Gray coverage retains
raw words after finite-unit validation. Ordinary entry and exit retain the
existing coverage rules, including Gray8 exit quantization and the all-white
sentinel. Presentation Color Indicates does not invert stored coverage. Current
indexed/imported source contexts and wide depth mismatches remain explicit
unsupported cases. These implementation changes have not completed runtime
acceptance under the implementation-first execution order.
Save Selection (`select.saveSelection`) and Load Selection
(`select.loadSelection`) accept the captured target pair. Save captures the
active selection source and the separately selected destination tab; both must
remain unchanged through preparation. Same-depth native Gray/no-alpha selection
copies retain raw words. An absent selection retains the existing zero-mask
Save behavior. Ordinary supported combinations keep the existing float32
arithmetic; unsupported wide combinations or depth changes fail before
extraction. The result still contains `channel`, `name`, and `document`.

Load Selection retains its default Gray8 selection quantization. Explicit
`nativeCoverage:true` requires `coverageContext:"unit"` and qualified plain native
saved-channel, Quick Mask, or raster layer-mask input with the same source/target
sample depth. Replace copies raw words; add/intersect use native max/min;
subtract and invert use checked unit arithmetic. Both native defaults and
retained samples must satisfy the unit policy. Native raw zero/signed-zero
selections stay stored (`selected:true`); the legacy zero result stays absent.
No implicit CMS conversion or imported filtered-channel interpretation is
introduced. Source and target snapshots/revisions/jobs are checked again before
publication. The captured dialogs resolve saved alpha choices by canonical ID,
so duplicate display names do not select a different channel.

New Channel and New Spot Channel also accept the captured pair and prepare their
native canvas coverage privately. Channel metadata management keeps raw samples
shared; rename/reorder/delete now reject stale or malformed targets before
normalization. Imported deletion with retained dependencies is unsupported.
The integrated implementations above passed native and WASM compile-only
checkpoint 73. Tests and runtime/source acceptance remain pending.

Actions sit in **set folders** (`set` on `actions.list` / `actions.get`, filled by Load Actions,
written by Save Actions, changed with `actions.assignSet`): the panel shows loose actions first,
then each set as a collapsible folder (`ui.actions.collapsedSets`); dropping an action among
another set's rows moves it into that set. Each step has Photoshop's two toggles: the check
(`actions.toggle`, skipped when off) and the **dialog** toggle (`actions.dialog`). Playing a
step with its dialog on opens that command's dialog seeded with the recorded values and pauses:
the play result carries `paused: {step, id, dialog}`, OK runs the dialog's values and continues
from the next step, Cancel stops the action. Agents (`automation_input`) play headlessly with the
recorded values. The panel menu's **Insert Stop…** adds an `actions.stopMessage` step after the
selected step (`actions.insert` takes any command id and params). `actions.conditional {if, then,
else}` is Photoshop's Insert Conditional as a step: it tests the document or active layer
(`always`, `landscape`, `modeIsRGB`, `hasLayers`, `isBackground`, `hasPixelMask`… see its params)
and plays the matching action by name, failing the caller when that action fails.

Layer-panel **Quick Export as PNG** exports every selected layer as a separate PNG. A single
Save dialog chooses the first filename; remaining files are numbered siblings with sanitized
layer names. Selection is captured before opening the dialog; an unselected layer is excluded.

## Shared panel docking

Panel arrangements support tab grouping and reordering, horizontal and vertical splits,
and movable, resizable floating groups inside the application window. Drag a panel tab,
or use its context menu. Floating groups remain within the application viewport.

The shared layout begins with the existing workspace on the first docking operation.
Workspace saves retain its groups, active tabs, split sizes, floating geometry, and the
positions of hidden panels. Resetting a built-in workspace restores its default dock.

- `window.panel.move {panel, anchor, zone?, before?}` moves a panel into the target group.
  `zone` is `tab` (default), `center`, `left`, `right`, `top`, or `bottom`; `before` reorders
  tabs by panel ID. The destination must already be visible.
- `window.panel.float {panel, x?, y?, width?, height?}` detaches a panel.
- `window.panel.dock {panel, anchor?, zone?, before?}` returns a panel to its previous
  dock placement when possible, or to the supplied destination.
- `window.panel.activate {panel}` reveals and selects a panel, restoring its saved
  position when possible. `window.panel.close {panel}` hides it.

Panel IDs are stable across themes. Invalid IDs, non-finite or invalid geometry, and
invalid destinations return an error without changing the arrangement.

Lock Workspace prevents moving, detaching, and closing panels; activating a panel remains available.

`window.panel.layout {action}` also accepts the shared, externally tagged action schema.
For example, `{"action":{"MoveFloating":{"panel":"properties","rect":[44,66,360,450]}}}`
updates a floating group's position and size; `{"action":{"ResizeSplit":{"path":[],"size":{"Ratio":0.35}}}}`
resizes the root divider (`false`/`true` path entries select first/second children).
`Move` with `placement: {"Tab":{"before":"swatches"}}` reorders tabs, and
`SetStackOpen {panel,open}` / `ResizeStack {panel,height}` control accordion entries.
The UI dispatches these same commands. Unknown panel IDs, invalid geometry, and stale
split paths return an error without changing the workspace.

The existing `ui.set` panel visibility, `dockTabs` selection and `dockWidth` controls
remain available after customization. Width keeps its 250–520 point clamp. Named
`dock.hidden_tabs` and intact native-group collapse settings are also preserved. A
legacy `dock` replacement that changes flat group order or heights is rejected after
customization: use `window.panel.layout` to change a custom split tree. A replacement
that changes legacy collapse settings is also rejected when a custom root group mixes
panels from different native groups; use `window.panel.group` to collapse or expand
that group. Validation is atomic across mixed `ui.set` requests.

`window.panel.group {panel, operation}` applies `close`, `collapse`, `expand`, `up`,
or `down` to the panel's entire current group. With `operation:"move"`, provide
`anchor` and optional `zone`/`before` (as for panel.move), or a serialized `placement`,
to transfer all members atomically while preserving their order and selecting the
requested `panel`.
The floating group's header grip also transfers the whole group when dropped on a
dock target. The panel menu and icon rail remain
available in custom workspaces. `window.panel.<id> {show:true}` always reveals, selects
and expands the panel, including when another tab is selected. Workspace snapshots
retain custom collapse settings and Timeline visibility.


## Stored saved-channel Load from an explicit original

The native desktop control handler and Headless automation backend accept a
host-only form of the existing `select.loadSelection` through `engine.execute`.
Native launch requires `--automation-read-root`; Headless construction requires
`Headless::with_workspace` with a held read root. Denied and TrustedLocal backends
have no ambient fallback. Platforms without the native host service fail closed.
Ordinary Load requests without `originalPath` retain their behavior. Native requests
borrow the live Session and use the same bounded facade; they do not move or clone
the Session, open a picker, normalize a transient preview, or run UI script hooks.

```json
{"id":22,"method":"engine.execute","params":{"command":"select.loadSelection","wait":true,"params":{"documentId":"1","expectedRevision":"0","channelId":"2","rawStorage":true,"storedSource":true,"originalPath":"originals/own-channel.xcf","originalOrdinal":"0","profilePolicy":"originalExact"}}}
```

The inner request contains exactly these eight fields. `documentId` and
`channelId` are canonical nonzero decimal U64 strings. `expectedRevision` is a
canonical decimal U64 string, allowing zero but excluding the exhausted maximum.
Use inspected identities and `revisionString` verbatim. `originalOrdinal` is the
original file's zero-based physical channel-record ordinal, a canonical decimal
string below 10,000; a current channel ID, index, name or tattoo cannot substitute
for this association. Both storage flags must be `true`. `wait:false`, wrong
field types, unknown keys and pending jobs refuse before synchronization or read.

CLI `run` accepts the same inner request with an explicit read root:

```sh
photocraft-cli run current.pcraft --automation-read-root ./inputs --cmd select.loadSelection --params '{"documentId":"1","expectedRevision":"0","channelId":"2","rawStorage":true,"storedSource":true,"originalPath":"originals/own-channel.xcf","originalOrdinal":"0","profilePolicy":"originalExact"}'
```

Replace the example identities and revision with the actual current document
values. The CLI validates this host request and requires the read-root flag before
opening its input document. The separate held root authorizes only the explicit
original read; ordinary CLI input, output and commands retain their existing
behavior. Unsupported original/current bindings still refuse through the same
facade, and this option does not grant a renderer or broaden XCF import support.

`originalPath` is an explicitly requested path beneath the already held read
root, following the workspace capability path rules. It must contain 1..4096
UTF-8 bytes without NUL. No document locator or SourceIdentity path is consulted.
The held regular file is read through one capability handle, in bounded chunks,
with a 16,384-byte limit and an EOF growth probe. Length/mtime changes, short
reads, excessive Interrupted retries and cancellation refuse before publication.
Cancellation is cooperative around filesystem calls.

Choose `profilePolicy:"originalExact"` for exact retained interpretation, or
explicitly request `"materializedImplicit"` for the existing closed RGB/nonlinear
original whose current ImplicitSrgb marker and ICC bytes pass the generated-profile
verifier. An absent original ICC alone supplies no working-profile authority.
Only the existing observed original-byte and canonical current-channel domain is
accepted; this path does not render or bake Gaussian appearance or apply its mask.

The backend holds a 384 MiB resident, 16 MiB index and 2^34 work aggregate.
Reader/path/view/header reservations reduce the engine's remaining envelope.
The original decoder phase uses 64 MiB resident, 2 MiB index and 16,000,000 work;
current proof uses 128 MiB resident, 4 MiB index and 2^32 work; publication uses
128 MiB resident, 4 MiB index and 2^31 work. All phases and actual capacities must
fit together. These lower limits do not enable a large-document policy.

The target snapshot and view are captured before reading and fenced again before
registered Load. The independent original decoder supplies the original node;
using a same-ID current node does not supply the proof. A successful result is
`{"selected":true}`. The registered command is authorized and executed once;
its journal receives only the existing five keys (`documentId`, `expectedRevision`,
`channelId`, `rawStorage`, `storedSource`). Original paths, bytes and grants are
not persisted. Replaying that five-key request without the scoped host offer
refuses. Native source/cache/filter state remains unchanged.

Owned XCF neighborhood continuation uses the existing command and native-save
interfaces for nine closed runtime domains: original/finite Box Blur and Median,
finite Gray Median, original Motion Blur and Pixelize, and original/finite Gaussian.
The renderer evaluates the existing neighborhood kernels over their actual
extents, including halos, under shared resident and work limits. Source and
native adoption validate the complete current recipe before publication.
Existing geometry, precision and implicit-profile restrictions still apply.
The owned path preserves enabled or disabled activation while validating the
complete typed recipe in either state. A disabled filter produces native
identity at the input extent, with no kernel, pixel CMS conversion or halo.
The existing `xcf.filter.enable` command supplies undo/redo and native continuation.
Source XCF remains read-only; native continuation uses `.pcraft`.
This implementation has compilation checks only; behavioral acceptance is pending.

The original Gaussian domain uses the existing 96-by-80 opaque RGB U8 nonlinear
source case, exact implicit profile, saved filter mask, stored compatibility
tuple and five arguments. Source selection requires enabled activation and the
original parameter intersection: horizontal standard deviation 2.25 or 3.75,
vertical 1.5, Auto filter, Clamp boundary and clipped extent. CURRENT edits use
the existing filter set/enable commands, paid inspection and native continuation.
Its evaluator preserves the existing source-to-linear conversion, Young
recurrence, inverse conversion and opaque-alpha rule. Constructor-owned
requirements price recursive scratch, lines, encoded writes and growing maps
before the actual quota reservation and kernel call. Disabled CURRENT state
validates the complete recipe and returns raw identity without executing the
kernel. This does not admit independently saved disabled originals, different
geometry, profiles or Gaussian policies through this source route.

The explicit owned source and native routes also retain the existing isolated
small-Auto Gaussian group intersection: a 16-by-12 RGB F32 nonlinear document,
mode-28 isolated group with two native children, a native backdrop, a physical
group mask and a distinct empty filter mask. The actual hierarchy and three
native leaves remain separate model objects in the native `.pcraft` project. Rendering
uses the existing child composition, Gaussian and group-mask order. This typed
route supports paid project/filter inspection, title copies and the existing
group activation transaction. Both `xcf.filter.set` and `xcf.filter.enable`
accept `{"filterId":"123","enabled":false}` for that filter. Pure activation adds
one `Set GIMP Group Filter Enabled` undo step; a no-op preserves redo. Inherited
content locks and complete current/proposed validation apply before publication,
and a saved disabled-only raw tuple cannot be enabled by rewriting it. Dispatch
is synchronous.

For that owned CURRENT group, `xcf.filter.set` also accepts one or two unique
`std-dev-x`/`std-dev-y` arguments as exact `Number.Float32Bits` values, with
optional `enabled`. Each axis must be finite and within `0..=15`; signed zero
is retained. For example:

```json
{"filterId":"123","arguments":[{"name":"std-dev-x","value":{"Number":{"Float32Bits":0}}}],"enabled":true}
```

Parameter requests add one `Set GIMP Group Filter Parameters` undo step even
when activation changes in the same request. Raw no-ops preserve redo. Empty
argument arrays, duplicates, opacity and changes to Auto/Clamp/clip fields
refuse. An enabled both-zero Gaussian still follows the conversion/filter/
inverse schedule. The original source and legacy parameter gates stay strict;
native `.pcraft` continuation retains the CURRENT values. Broader numerical
acceptance remains pending.

The same owned class accepts combined `layer.setProps`
name/visible/opacity for its four layer IDs through the existing metadata
transaction, including exact coalescing keys and first undo labels. Physical
group-mask activation uses `xcf.mask.enabled.set`; its mask lookup excludes the
distinct filter-mask ID. The filter mask retains its empty-selection semantics.
Current and proposed native/default/mask/profile state validate even for hidden
or disabled output. Mode 61, other Gaussian arguments and broader source
geometry/profile policies remain outside this route. Source XCF is read-only;
save/reopen continues in `.pcraft`. Behavioral acceptance remains pending.

The explicit keep-current owned XCF route also selects a separate assigned-ICC
geometry family for the existing Normal source intersections: nonlinear
RGB/Gray U16/U32/F32/F64 and Linear/Perceptual F32/F64. It retains two native
layers, offsets, opacity and one ordinary layer mask. The original encoding and
float/integer geometry epochs remain distinct. A shared paid profile owner
derives the current definition from the retained original ICC and checks its
encoded bytes; rendering uses the generated definition to preserve analytical
transfer constants. Strict source and native handoffs validate the current
profile and complete canvas before publication, under the same quota and
cancellation/work control. Native title changes preserve
the shared mask and source owners. Paid `xcf.inspect` exposes the three items
and mask metadata. The existing `layer.setProps` command accepts combined name,
visibility and opacity changes for either layer through the paid imported
metadata transaction. It retains upstream exact coalescing keys, undo grouping,
cast/clamp and unchanged-value behavior, with current and staged profile/canvas
validation before publication. These edits require synchronous dispatch and
share the registered finite work ledger; other layer/filter/selection edits
remain outside this closed family. The foreground retains its exact Union, Clip to Backdrop, Clip to
Layer or Intersection policy, with a Union backdrop. Typed composition handles
outside-layer and zero-opacity coverage; a hidden layer skips composition.
Existing bare-profile constructors retain their Union scope. Other blend modes,
profiles, editing domains and behavioral acceptance remain pending.

For the same assigned geometry family, `xcf.mask.enabled.set` accepts the
existing strict `{"maskId":"canonical decimal U64","enabled":bool}` request
and returns `maskId`, `layerId`, `enabled` and `changed`. It changes only the
native and canonical current enabled flags, retaining the exact mask samples
and source owners. Owner and mask content locks apply before no-op detection.
Current and staged profile/canvas validation precede one undo step; unchanged
requests preserve redo. Disabled masks still undergo complete validation.
The command remains synchronous, rejects coalescing and has no journal entry.
Headless nonwaiting requests retain their existing synchronous-dispatch refusal.

The explicit keep-current owned route also retains the existing isolated Normal
group family: one group with two editable raster children above a backdrop,
RGB/Gray U16/F32/F64 nonlinear assigned ICC, and an optional group mask. The
original 17-by-13 source predicate includes hidden-group and disabled-mask
controls. No group projection replaces its children. Source-selected staging
and consuming promotion retain the three actual native leaf owners and mask
receipts. Strict source/native handoffs validate the exact original/current
profile relation and complete canvas before publication, including hidden and
disabled content. Paid title changes, native continuation and `xcf.inspect`
preserve the hierarchy and its four or five canonical items. Group rendering
reuses the existing isolated Normal equations with paid band overlap and caller
cancellation. Other group modes and deeper hierarchy remain outside this
owned family; tests and behavioral acceptance are pending.

Within that group family, existing `layer.setProps` supports combined name,
visibility and opacity changes on the backdrop, group or either child, including
reverse depth-first name selection. It shares the existing exact coalescing
keys, first undo label, changed-visibility locks and no-op redo behavior. The
optional group mask uses the same `xcf.mask.enabled.set` transaction and original
reply, context, lock and journal rules. Current and staged group profiles/canvas
are validated before publication, with unchanged native/mask/source custody.
Typed CURRENT composition permits backdrop visibility and opacity changes;
bare constructors and original source qualification retain their stricter gates.
Other property, mask, geometry and group-mode edits remain outside this family.

For plain imported native layers with an implicit RGB or Gray working profile,
the existing `layer.translate` command supports whole-layer integer movement.
It shares upstream pixel/point parsing and rounding, hidden-target checks,
effective position locks, `Move` undo labels and null replies. An exact opaque
`coalesce` key shares the document's existing metadata/Move gesture and retains
its first undo label; rounded-zero movement preserves model and redo state.
Both current and staged native/default data and complete canvas rendering are
validated under the same quota and work control before publication. The backing
view and canonical current raster bounds move together, retaining original
source pixels, profile and source protection. This path requires synchronous
dispatch, with no pixel selection, mask or linked-layer expansion. An explicit
layer ID or name overrides selection; otherwise all selected plain layers move
in document order. All targets pass validation before one model/history
publication. A retained effects reference point uses the same per-axis addition
as upstream movement. Native continuation uses the existing current
descriptor and tile writer. Behavioral tests remain deferred.

For the existing assigned-ICC Normal geometry family, `layer.translate` also
moves the foreground with its linked mask. The backdrop stays fixed. A linked
`"target":"mask"` request takes this same whole-layer path, as upstream does;
disabled linked masks still move. Checked raw mask staging retains complete
stored tiles and exact defaults under the actual quota and work control.
Current canonical layer and mask bounds and mask offset move atomically with
the actual pixels. This current-state synchronization is explicit; ordinary
resident reconciliation retains declared mask bounds. Original source evidence,
profile, mask flags and operators remain unchanged. Complete current and staged
assigned profile/native/mask/canvas validation precedes the shared undo and
gesture transaction. Independent unlinked-mask, backdrop and group movement
remain outside this path. These additions have compilation checks only;
behavioral acceptance is pending.

The same effects-reference positioning applies to the assigned foreground
movement above. Zero-vector movement preserves its original bits; any nonzero
move performs the upstream addition on both axes, including a zero delta on one
axis. Plain bulk movement preserves unrelated layers and active/selected focus,
creates at most one undo step and shares the existing exact gesture key and
first label. Any target lock, invalid extent, cancellation or quota failure
leaves the entire command unpublished. Assigned foreground movement continues
to require a single selected layer; groups and linked-layer expansion need
their own qualified transactions.

The explicit owned flat-composite route now preserves the qualified 21 modern
separable blend modes and four non-Lab explicit Multiply space cases. Source
staging selects one family and retains the actual codec/native receipts. Canvas
rendering uses the shared blend equations with prospectively paid requested RGB
working spaces and bounded bands. Current native/default/profile data is fully
validated even for hidden content. Canonical composition metadata determines the
operation; physical blend enums remain inert, including grain-mode surrogates.

This first route retains the existing two-layer F32 nonlinear implicit-sRGB
source intersection. Native continuation, paid document-title changes and
`xcf.inspect` project replies are connected; project inspection stays limited
to two items, no filters and at most one selected item. The structural runtime
class grants no broader source, render or query eligibility. Generic metadata,
Move, pixel and selection edits require separate transactions. Tests and
behavioral acceptance remain deferred.
For that owned flat-composite family, `layer.setProps` now accepts compound
name, visibility and opacity changes on either leaf through the existing
transaction. Exact gesture keys preserve the first undo label, and unchanged
requests preserve redo. Native/canonical names and visibility stay coherent;
opacity retains its F32 bits, including signed zero. Complete current and staged
profile/native/canvas validation precedes publication under the actual quota and
work control. The synchronous UI route applies even with background jobs enabled.
A separate metadata-only routing hint preserves mask and Move restrictions.
Original source gates stay unchanged; behavior tests remain deferred.
CURRENT flat-composite rendering now accepts 1–4096 independently positioned
native leaves and arbitrary checked canvas bounds within its retained F32
nonlinear implicit-sRGB domain. Original XCF source enrollment stays at the
existing exact family. Known stored Auto fields retain their version/mode/epoch
rules. Fully explicit CURRENT settings use the existing composition command's
supported RGB space/operator/mode subset. Unsupported advanced blending controls
return an error. Complete native/default/hidden/off-canvas validation and bounded
two-axis canvas evaluation precede publication.

`xcf.inspect` now pages flat-composite items and selections independently using
the existing offset/limit schema: default offset0/limit64, maximum limit128,
offset up to100000, and valid empty pages beyond the total. Paid page facts and
reply bounds cover only returned rows; conservative full canonical text work is
still charged. Explicit all-Normal modern flat native stacks select the strict
CURRENT endpoint; ordinary Auto Normal and mixed legacy Normal keep their prior
route. These additions do not enable composition mutation or expand source
import rules. Behavioral tests remain deferred.
For owned flat-composite documents, `xcf.composition.set` uses the same existing
wire and mode subset as resident composition editing: one of20 modern ordinary
mode equivalents, `legacy:false`, explicit RGB blend/composite spaces, and one
of the four operators. It retains the compatibility epoch and original source
records while synchronizing current physical and canonical policy. Grain modes
46/47 have no invented ordinary alias; an existing grain layer may explicitly
transition to a supported mode. The owned composition panel offers these modes;
ordinary resident mode presentation stays unchanged.

Optional `documentId` and `expectedRevision` remain a paired captured-target
fence using canonical decimal strings. Revision exhaustion and content locks
refuse even a no-op. An Auto-to-explicit transition creates one undo step even
when effective math agrees; repeating exact explicit policy preserves model,
revision and redo, while the existing reply still reports `updated:true`.
Full current/staged native/profile/canvas checks, prospective copy/history/view,
retained reply/journal fees and final source/state fences precede publication.
UI and Headless use synchronous dispatch for this actual class. Source import
rules are unchanged; behavior and UI visual checks remain deferred.
Original flat modern XCF source selection now has an additional bounded route
for 1..4096 flat RGB F32 nonlinear native leaves using implicit builtin sRGB.
It preserves original raw policy words and uses an exact version/tuple table
for witnessed Auto policies plus the existing explicit20 mode subset. Hidden,
zero-opacity and off-canvas native content still receives full validation.
Existing strict source cases keep priority; refusal cannot retry through this
route, and all-Normal source files retain Plain selection. This route uses the
same owned FlatComposite storage, paid rendering, inspection and native .pcraft
continuation. Assigned ICC, masks, effects, groups and other unsupported domains
remain outside this intersection. Ten draft regression groups remain unrun;
independent GIMP profile-byte and numerical observations do not establish
PhotoCraft acceptance.
Owned FlatComposite layers also use the existing `layer.translate` command.
Selection, explicit target, px/pt deltas, hidden-target handling, position locks,
gesture coalescing and raw no-op behavior follow the existing Move path. Every
requested fixed-root native coordinate view is checked before one paid model
copy; current canonical bounds and existing effect-reference arithmetic move
with it. Source pixels, defaults and original composition records remain exact.
Masks and linked-layer expansion are outside this route. Desktop dispatch uses
the synchronous transaction even when background jobs are enabled; Headless
retains its existing wait/auth rules. Behavioral checks remain deferred.
The owned flat-composite path now also carries fourteen channel-separable
legacy modes: 3,4,6..10 and15..21. Original source admission is a separate strict
v12 RGB F32 nonlinear implicit-sRGB two-layer intersection with the established
raw policy/geometry and complete finite-unit native checks on both planes.
Its refusal cannot retry through General or Plain. CURRENT recipes use bounded
dynamic layer geometry, exact legacy flag/epoch/raw/resolved policy, and full
unit validation of every legacy native/default sample, including hidden and
off-canvas content. Existing legacy min-alpha coverage, ratio endpoints and
128/255 grain behavior remain the evaluation authority. This adds no legacy
composition mutation wire; explicit composition edits keep their existing
modern20 subset. Behavioral and numerical acceptance remain deferred.
Additional owned RGB blend paths cover legacy HSV/HSL11..14, modern HSV/HSL37..40,
and luminance modes54..56. Exact v12 raw tuples retain Profile/Profile legacy
coverage, Profile/Linear modern component coverage, and Linear/Linear luminance
coverage. Original source admission retains the established strict two-plane
geometry and both-plane finite-unit checks; the new Component route cannot
retry refusal through General or Plain. CURRENT retains dynamic bounded geometry
and checks affected native/default inputs even hidden or off-canvas. Existing
D50 profile luminance weights, equal-Y/different-RGB tie refusals, zero-backdrop-Y
refusal and finite HDR Luminance results remain unchanged, with no new clamp.
The existing modern20 composition mutation wire remains the available explicit
transition path; these additions do not add component or legacy mutation modes.
Tests and numerical acceptance remain deferred.
Owned alpha/color modes57..60 have separate exact v12 RGB and Gray source
routes using the established two-layer geometry and complete finite-unit native
validation with four or two channels. Current Gray binds actual BuiltinSGray
bytes and ImplicitSGray interpretation and permits only Normal28 plus at least
one alpha mode; other blend families retain their RGB gates. Existing alpha-zero,
hidden-RGB, exterior zero-source and output-alpha behavior remains unchanged.
Native Gray storage remains two channels; display replication is separate.

Gray composition mutation is unavailable in the typed model API and registered
engine path before no-op, including explicit Normal changes on the background
layer. Its composition UI stays read-only. RGB retains the existing modern20
transition wire; metadata and Move use the same paid transactions in both models.
Eight referenced producer Gray effective-ICC captures are missing locally and
remain an evidence HOLD; no byte equivalence is asserted. Tests, UI visual checks
and numerical/profile acceptance remain deferred.
Owned LCh modes24..27 use physical CIELab D50 through a separate paid preparation
owner. Before initialization, the caller prepays bounded planning, the full
constructor peak and constructor work for effective-source/target profiles,
both Relative/noBPC transforms and physical side pipelines, including stage
vector overlap. Actual per-pixel pipeline work is charged before both exterior
zero-source and operative evaluation. Existing signed inverse RGB and zero-chroma
behavior is unchanged; normalized ICC Lab is not a fallback.

Original LabStrict source selection retains the established v12 RGB F32 nonlinear
implicit-sRGB two-layer geometry, raw(-3,-1,-2) and complete native-unit proof,
with no refusal retry. CURRENT remains bounded dynamic geometry and requires
exact Lab policy. The original physical import hint remains absent; canonical
composition carries Lab authority. Existing RGB modern20 explicit transitions
may replace it with their matching physical hint. No LCh composition setter,
new profile grant or native normalization is added. Tests and numerical/profile
acceptance remain deferred.
Owned assigned-profile F32 Box Blur uses a distinct readonly runtime class
and dedicated source/CURRENT validators. Both actual ICC roles remain owned;
controlled parse/analytic qualification and generated-current encoded equality
precede rendering with three independent transfer curves. The assigned canvas
route ignores caller-supplied profile placeholders and fails finally. Native
reopen uses an explicit consuming handoff followed by full current validation;
serialized fields grant no runtime authority. Hidden/disabled content still
receives complete native/default/mask/profile validation. Title and paid read-only
inspection are supported; generic filter/metadata/Move/selection/pixel edits are
closed for this foundation slice. Old implicit Point admission remains unchanged.
The named unit-F32 samples contain five out-of-unit RGB words, independently
decoded and retained without clamp. The new Box gate follows the existing finite
RGB/unit-alpha equation domain; every other source/profile gate stays exact.
Four missing loose producer ICC captures remain HOLD. Tests and numerical/profile
acceptance remain deferred.
Assigned-profile F32 Median now shares the paid original/generated-current
profile relation with dedicated exact source/CURRENT constructors. Its existing
encoded quantized/high-precision kernels are unchanged, with finite RGB/unit
alpha and complete native/default/mask validation. The actual-class dispatcher
selects Box or Median with no retry; public Box-only constructors stay exact.

Assigned Box supports enable-only changes through the existing xcf.filter.set
and xcf.filter.enable commands, each with exactly filterId/enabled. Complete old
and staged validation, locks, captured state, actual history-current equality,
finite work and existing paid publication fences precede commit, including no-op.
UI exposes its activation checkbox and labels parameters read-only. Median
activation, arguments, metadata, Move and other edits remain unsupported here.
L1 activation patches are structural data operations; engine qualification
supplies command authority. Save/reopen continuation uses native .pcraft files;
XCF writing is outside this feature. Tests and visual/numerical/profile acceptance
remain deferred, and missing producer ICC captures remain HOLD.
Assigned-profile F32 Gaussian now has dedicated source/CURRENT preparation and
joins the authoritative Box/Median/Gaussian current dispatcher. Four actual
named-unit/HDR source/edit files use the exact existing five-field Auto/FIR
contracts, finite RGB/unit alpha and full native/default/mask/profile proof.
The generated current profile is bound to the retained original; actual profile
peaks and working-transfer construction are paid through execution. Existing
whole-axis Gaussian kernels, requirements and numeric schedule are unchanged.
Box/Median-specific constructors remain exact. Gaussian filter edits, including
same-value activation requests, remain unsupported. Native .pcraft saves/reopen,
title and paid inspection use the existing assigned storage class. Eight missing
producer ICC captures remain HOLD. Tests, numerical/profile and visual acceptance
are deferred; this does not establish general image geometry or lazy source IO.
CURRENT/native float Box now uses bounded canonical-tile rendering over checked
arbitrary drawable/canvas geometry. Its exact old radius4/5, raw/schema and
implicit RGB F32/F64 or assigned RGB F32 profile contracts remain authoritative.
Every full native/default/mask/profile check still precedes hidden/disabled
shortcuts. Implicit Point radius edits and assigned Box enable-only transactions
use complete old/staged validation; their wire and locks remain unchanged.
Original XCF source admission remains separate and unchanged.

The kernel clamps to original drawable S and expands only output O. Full global
256-pixel tile parts complete before cropping a requested band. Private packed
input/kernel scratch is guarded at16MiB, and Box preparation/current-band/scratch
at32MiB including wrapper headers/profile peaks. Persistent input/provider cache,
previously returned output and generic reduced-preview caller buffers retain
actual same-quota leases outside that child temporary guard. This is not lazy
XCF input or a total-process memory bound. Full24–36MP enabled proof currently
exceeds normal4.295b/8b caller work allowances; bounded work admission remains
unfinished. Twelve new regression groups are UNRUN/uncompiled. Tests and
numerical/visual/performance acceptance remain deferred.

Original-XCF Box geometry now has a separate observational compatibility epoch,
xcf22-observed-box-geometry-v1. Implicit RGB F32 nonlinear650 uses checked generic
canvas/drawable bounds, enabled radius4/5, Replace62 at unit effect/layer opacity,
region/clipping zero, no auxiliaries and an actual canvas-sized numeric-zero
physical filter mask. Three present stored effect-policy i32 words remain opaque
provenance with resolvedNone; they are never normalized or interpreted as enum
permissions. The old16x12/origin+old-triple identity retains route priority and
final refusal. Other supported identities are selected before qualification;
there is no failure retry. Independent GIMP observations support this deliberate
closed compatibility policy, not a universal statement about arbitrary XCF enums.

The new epoch supports paid inspection/viewing and native .pcraft continuation.
Accounted mutations are refused before helpers/no-op; layer selection and history
navigation remain separate. Original native/default/profile/physical-mask proof
and postpromotion full expanded-output proof remain mandatory. Source preparation
retains the decoded Surface and paid profile/working-space state, without a full
filtered Surface. It makes no lazy-source or total-memory claim.

Box host rendering, registered set/enable for existing editable epochs, and
native/XCF adoption now select the spec's finite standard work policy: native2^40
or wasm2^36 units, with120s active deadline or an explicitly lowered helper limit.
One cumulative used count/clock/cancellation owner spans preparation, current and
staged proofs, reply and publication. Provider work is never discounted; external
ReadControl entrypoints remain authoritative and unrelated registered caps stay
unchanged. Browser picker wait precedes the first active clock. Native/XCF intake
still uses legacy8b bootstrap until the actual Box owner is available, with no
selected120s enforcement in that earlier phase; selection retains elapsed time
and immediately refuses an expired operation. Native logical extent limits,
decoded input costs, lower wasm allowances and numerical/performance acceptance
remain unfinished large-image boundaries. New regression groups are UNRUN.

The observational Box geometry epoch now includes implicit RGB F64 nonlinear750.
Actual native32-byte pixels and GrayF64 mask8-byte samples are scanned without
narrowing; current source/default/mask/profile checks, opaque raw policy, readonly
mutation boundary and old-identity final refusal remain. GIMP F64 captures retain
exact native/output roles; two effective ICC captures differ only in date-header
bytes33/35 and remain separate immutable evidence. No source ICC is normalized.

Owned native save/reopen now share max_finite_logical_bytes in OwnedDocumentLimits:
a per-raster default/hard2GiB descriptor allowance, caller-lowerable including
zero. It replaces the old private256MiB save/reader constants without allocating
a whole raster or supplying additional resident/encoded/index/tile capacity.
Box UI/Headless bounded Save/Save As also selects the same finite standard work
and active deadline before snapshot/catalog/writer. Actual parent cancellation,
source exclusions, destination/atomic writer, history/journal/publication fences
and external caller controls stay authoritative. Other bounded saves retain
legacy work limits. Encoded native256MiB/browser16MiB intake and preselection
work/deadline boundaries remain; no dense24–36MP or native_large completion is
claimed. Nine new regression groups are UNRUN/uncompiled; runtime/visual/numerical
and performance acceptance stay deferred.

Observed Box geometry now includes assigned RGB F32 nonlinear650 through the
actual readonly assigned Point owner. Source preparation and CURRENT rendering
reuse the existing controlled original-to-generated-current profile binding,
retain both ICC owners/peaks and use one Linear working space. Full physical
mask/native/default/profile proof remains mandatory; raw words remain opaque,
old identities retain deterministic priority/final refusal and this epoch's
mutations remain closed. No new profile class, kernel math or CMS transfer is
introduced. Independent source/native/profile/alpha words are exact. GIMP fresh
straight RGB varies near small alpha; preserved-sample premultiplied and
black/white comparisons are small, but this is not matched-display/CMS or
PhotoCraft qualification. SC-002 and all acceptance tolerances stay unchanged.

All explicit owned keepCurrent native/XCF intake now selects the declared
standard resource work/deadline policy before input access, independent of file
content. One used count, original cancellation and lazy active clock span source
capture/decode, validation, response and adoption. Browser picker wait precedes
first actual label/capture work. Late Box reselection is removed. Caller-supplied
ReadControl routes, ordinary non-owned Open and unrelated registered caps remain
unchanged. This supersedes the earlier8b/unenforced-deadline bootstrap notes for
these explicitly selected intake routes.

Encoded backing/capture ceilings now match the existing platform standard:
1GiB native and256MiB wasm. The same native-store constants constrain selected
paths, held roots, memory/persistent captures and browser bytes; lower source/
block limits remain authoritative. Persistent digest indexes, Scratch/Resident
and browser provider/Rust overlap are still actually reserved before reads/fill.
Browser source preparation retains its lower resident128MiB/index16MiB/scratch0
limits, now with platform encoded/logical byte bounds. These ceilings promise
neither full fit nor dense large-image throughput and do not enable native_large.
Fourteen new groups remain UNRUN/uncompiled; numerical/visual/workflow acceptance
and PhotoCraft test/runtime execution remain deferred.

The observed Box geometry epoch now supports radius0..1000 in original enabled
sources and CURRENT editing, superseding its earlier readonly/radius4/5 notes.
Both actual implicit RGB F32/F64 and assigned RGB F32 owners use the existing
xcf.filter.set and xcf.filter.enable transactions. Set accepts exactly filterId,
one existing radius Signed32 argument and/or enabled; other arguments, opacity,
unknown keys and other mutations remain refused. The owned panel exposes the
same radius and activation controls through the existing checked dispatch.
Current/history equality, locks, full current/staged native/default/mask/profile
proof and paid revision/history/view/source/journal/reply fences precede even
same-value acceptance. Assigned data copying preserves both ICC owners and the
original source; its structural Signed32 API does not authorize an operation.

Other positive radii use bounded row recomputation;4/5 retain the old packed
path. Active0 still converts through Linear and inverses after its packed
working identity. Disabled filtering is distinct. Full canonical tiles complete
before cropping, typed source errors survive and all actual reads/conversions
are charged. Existing16MiB/32MiB temporary guards and standard operation caps
remain. Large/high-radius completion and numerical/UI/runtime acceptance remain
pending; all new regression drafts are UNRUN/uncompiled.

Observed Box owners now join the existing layer.setProps name/visible/opacity
transaction, including its upstream target, coalesce-key, drag/eye and undo-label
semantics. Both actual implicit and assigned Point classes use the same full
CURRENT/staged Box validation and paid gesture/history/view/source/channel/reply
fences. Direct compound Point metadata calls fail closed outside that exact
observed Box policy. Other filter families keep their existing registered paths;
no mask, Move, selection, native-pixel or composition edit is added.

Layer name and visibility are synchronized with their canonical item fields;
layer opacity retains its exact F32 bits. The filter's own Replace opacity,
arguments, raw words, mask and stack revision remain unchanged by layer metadata.
Original source admission remains visible/unit-opacity/enabled. Hidden or
zero-opacity CURRENT documents still receive complete proof. The standard Box
work/deadline policy applies to this narrow command route; all provider work is
charged and high-radius/large workflows may still reach its limits. New drafts
and runtime/visual acceptance remain deferred.

Observed Box CURRENT owners now join existing layer.translate through its shared
Move parser, targets, px/pt conversion, hidden-layer guard, native Position/All
locks, gesture key and first undo label. The synchronous UI/Headless routes retain
existing behavior. A nonzero edit requires the exact observed Box policy and
complete current/history/staged operative and canvas proof under one standard
Box work/deadline control. Other actual Point families cannot fall through to a
generic Move policy. Existing rounded/direct zero vectors remain mutation-free
upstream no-ops before target/lock/render proof; that grants no edit authority.

The data adapter retains the original native root and shifts only the operative
view, canonical layer bounds and existing effects-reference arithmetic. Canvas
filter masks and their offsets, nodes, raw words, stack revisions and both profile
roles stay unchanged. Linked/mask/selection/pixel editing is not added. Rendering
recomputes Box on the shifted world tile grid instead of shifting a filtered
bitmap. Original source admission stays unchanged; numerical, visual, native
workflow and performance checks remain deferred.

Original observed Box source admission now accepts visibility, finite unit
effective F32 layer opacity and enabled/disabled filter state for the existing
implicit RGB F32/F64 and assigned RGB F32 roles. This supersedes the earlier
visible/unit-opacity/enabled source-only restriction. The compatibility policy
uses the independent activity controls; it does not claim every activity/radius
combination was numerically qualified. Filter Replace opacity stays exactly1.

FLOAT_OPACITY tag33 is authoritative over compatibility tag6; both original
payloads remain intact. A .375 source can correctly carry legacy byte95 while
its float property is exactly0x3ec00000. Source staging does not quantize the
float or require equality to the byte divided by255. Hidden, zero-opacity and
disabled sources still undergo complete native/default/physical-mask/profile/
working-space checks and actual CURRENT proof before publication. Active0 CMS
identity remains distinct from disabled native identity. Old source identities
retain priority/final refusal and old specific/eager contracts stay exact.
Assigned producer RGB variability and effective ICC date-byte HOLDs remain;
PhotoCraft numerical/visual/runtime acceptance is still deferred.

Observed Box now also admits assigned RGB F64 nonlinear750 through an additive
source role and actual assigned native owner. Its native32-byte pixels and
GrayF64 mask8-byte samples remain exact, including finite HDR, signed-zero and
sub-F32-resolution controls; both original/current ICC owners retain the existing
controlled profile relation and paid preparation. There is no F32 narrowing or
new CMS/kernel math. Old assigned Box/Median/Gaussian specific/eager contracts
stay F32-only, and original old-identity selection/final refusal stays exact.

The new role intentionally shares the existing observed radius0..1000,
activation, layer metadata and Move command scope under the same full current/
staged proof and transaction fences. UI controls keep actual class/profile and
exact precision-pair eligibility. Source activity uses the existing explicit
compatibility policy; the independent F64 r4/r5/reopen controls do not establish
an exhaustive numerical theorem. Its captured profile roles are distinct from
unresolved older F32 evidence and replace none of those HOLDs.

Input/capture/work/deadline limits remain unchanged. Decoded RGBAF64 alone costs
768MB at24MP or1.152GB at36MP before padding, source/encoded storage, physical mask,
profiles and renderer overlaps. Browser128MiB and wasm512MiB resident ceilings
cannot fit those floors; native whole-workflow fit/throughput is unproven. Bounded
16MiB/32MiB renderer guards do not enable native_large or promise input fit.
New regression drafts remain UNRUN/uncompiled; PhotoCraft runtime, numerical,
visual and workflow acceptance remains deferred.

Observed Box additionally accepts implicit RGB Linear F32/600 and F64/700 with
no original ICC. The current effective Linear ICC and its byte-identity marker
remain distinct from the nonlinear builtin profile used as the preparation
basis. Import and current rendering use the actual stored encoding throughout
profile planning and working-space preparation. The same observed radius,
activation, metadata and Move transactions apply, with full current/staged
proof and existing source/history protections. Assigned Linear is not admitted.
Source profile construction is controlled and paid on the existing shared
budget; inactive and radius-zero paths retain their checks and charges.

Independent GIMP Linear source/projection/merged captures matched across live,
same-process, fresh and repeated reopen controls. Effective-profile date-byte
differences remain recorded separately from stable, distinct Babl profile bytes.
Those observations establish implementation inputs, not PhotoCraft numerical
or workflow acceptance. Tests and final gates remain deferred.

The observed Box envelope also accepts implicit RGB Perceptual F32/675 and
F64/775 with no original ICC, under the same radius/activity/metadata/Move and
native continuation policy. Stored Perceptual samples pass through the existing
Perceptual-to-Linear transfer before filtering and its inverse afterward.
Current effective-profile bytes and marker remain independently bound; equal
builtin transfer curves do not permit substituting nonlinear profile bytes.
The controlled source profile factory admits only the exact sealed Linear or
Perceptual precision pairs and retains the same prospective resident and
cumulative work limits. Assigned Perceptual remains outside the envelope.

Independent Perceptual source/projection/merged captures are bit-exact across
the live/same/fresh/repeat controls at both depths. Their effective ICC date-byte
HOLD and distinct Babl profile remain explicit. PhotoCraft regression drafts,
runtime, numerical, visual and performance acceptance remain deferred.

Observed Box additionally accepts implicit Gray nonlinear F32/650 and F64/750
with no original ICC, under the existing observed activity/radius/metadata/Move
and native continuation policy. Native pixels remain Gray+Alpha at 8/16 bytes,
alpha in channel 1; masks remain one channel at 4/8 bytes. Rendering repeats
Gray only in its internal RGBA working/output lanes, preserving native source
words, defaults and source ownership. Internal kernel memory remains fully paid.

The generic caller-basis check requires exact nonlinear SGray bytes and one
channel for Gray; the current effective ICC and ImplicitSGray marker still take
their independent controlled binding. Nonlinear effective SGray may have the
same encoded bytes as the builtin basis. RGB basis checks retain their previous
behavior. Assigned Gray and Gray Linear/Perceptual remain outside this envelope.

Independent Gray F64 captures are stable across live/same/fresh/repeat. Gray F32
source, alpha and merged captures are exact, while live-to-fresh projection Gray
words differ by up to 2.384185791015625e-7; fresh-to-repeat projection is exact.
This producer variability, effective Gray ICC date bytes and the distinct RGB
Babl space-profile role remain explicit evidence limits. PhotoCraft tests and
all numerical, visual, performance and workflow acceptance remain deferred.

The observed implicit Gray envelope now also accepts Linear F32/600 and F64/700
and Perceptual F32/675 and F64/775, with the same source/activity/edit/native
continuation policy. Their actual stored encoding selects existing SGray
working-space preparation and generated current ICC/ImplicitSGray identity;
native YA storage, masks and internal RGBA kernel fees remain unchanged.
Actual Gray canvas display selects sRGB encoding for Linear and retains the
Perceptual profile interpretation for Perceptual. Assigned Gray stays outside
the envelope.

Combined independent captures preserve a new Linear F32 variability limit:
both projection and merged Gray words differ by up to 5.960464477539063e-8,
including merged differences between fresh and repeated reopen captures.
Source and alpha remain exact; the other three roles are capture-stable.
Effective Gray ICC date bytes, distinct Babl RGB-space profiles and each actual
opaque raw triple remain provenance. No tolerance, profile normalization or
PhotoCraft numerical acceptance is inferred; regression groups remain UNRUN.

Owned Semi-Flatten continuation uses the existing `xcf.filter.set`,
`xcf.filter.enable`, `xcf.filter.remove` and singleton reorder transactions.
The new role is exact v22 `gimp:semi-flatten` / `0:0`, implicit RGB nonlinear
F32/650 or F64/750, equal checked positive native/canvas bounds at the origin, finite unit RGBA,
and its separate `xcf22-gimp-semi-flatten-literal-v1` epoch. Source import
requires the authored visible/unit-opacity layer, enabled filter and declared
zero selection mask. Complete current/staged proof and the existing paid
history, view, source, channel, journal and reply fences remain mandatory.

The sole `color` argument uses the existing tagged `Color` wire value: exactly
four `Float32Bits` components, `profile: null`, and `encoding` either `linear`
or `nonlinear_profile`. Both tags retain their own meaning and all four raw
words; the literal alpha is validated and retained even though it does not
affect the Semi-Flatten equation. Switching only the encoding is an edit;
exact words plus tag are a no-op. Linear colour requests are allowed only for
an actual owned Semi-Flatten node under the exact epoch. Existing generic
colour edits keep their nonlinear contract. UI RGB edits preserve the tag and
alpha words, and simultaneous colour/activation edits use one transaction.

The existing readonly `layer.setProps` adapter for layer name, visibility and
finite unit opacity also applies after complete proof, including simultaneous
supported fields. The direct compound metadata API and Move remain closed.
Removal uses a separate derived-current empty RGB F32/F64 origin role;
it does not enroll new empty original-XCF sources. Save/reopen here means the
native `.pcraft` continuation. Exact GIMP conversion/rounding, PhotoCraft
numerical results and all regression/native/visual/performance checks remain
deferred; the GIMP research captures do not complete those gates.


Owned Hue-Chroma continuation uses the existing filter transactions under
`xcf22-gegl-hue-chroma-lch-float-v1`. The first role is exact v22
`gegl:hue-chroma` / `0:0`, implicit RGB nonlinear F32/650 or F64/750, native and
canvas checked positive equal bounds at the origin, finite unit native RGBA and the separate
singleton Replace source/current policy. Exactly three unique stored arguments
use `Number.Float32Bits`: `hue` [-180,180], `chroma` [-100,100] and `lightness`
[-100,100]. Preserve raw words and stored order; no gdouble/F64 wire alias or
hue normalization is inferred. UI changes use bounded controls and one atomic
argument/activation transaction. Exact raw no-ops preserve existing history rules.

Existing set/enable, readonly name/visibility/unit-opacity metadata (including
combined supported fields), singleton reorder/remove, paid inspection and native
`.pcraft` save/reopen keep complete current/staged validation and existing fences.
The direct compound metadata API and Move remain closed; empty-after-remove is
the separate derived-current role, not a new original-XCF source identity.

The paid renderer uses an explicitly declared physical Lab/LCh Float recipe,
retaining generated Float alpha and finite HDR/negative RGB. Neutral enabled
filters still convert; disabled identity still receives full input/metadata proof.
GIMP F32 author/fresh projection/merged differences about 0.5086223 and separate
profile/rounding/default-space graph limits remain HOLD. No numerical acceptance
or tolerance waiver follows from source transport or a matching standalone graph.
All new regression and native/widget workflows remain UNRUN and uncompiled.


Owned Vibrance continuation uses the existing filter wire commands under exact
`gegl:vibrance` / `0:0` and `xcf22-gegl-vibrance-linear-lch-float-v1`. The
actual Point role is implicit RGB nonlinear F32/650 or F64/750, original ICC
absent, equal checked positive native/canvas bounds at the origin and finite unit native RGBA. Its separate
source/current Replace policy and full current/staged proof remain mandatory.
Exactly two unique arguments use `Number.Float32Bits`: `vibrance` [-100,100] and
`saturation` [0,10]. Saturation is a scale with default1, without percent or F64
aliases. Bounded controls combine changed arguments and activation atomically;
exact raw no-ops retain existing history rules.

Registered readonly name/visibility/unit-opacity edits (including combined
supported fields), singleton reorder/remove, inspection and native `.pcraft`
save/reopen retain all existing source/history/view/channel/journal/reply fences.
The direct compound metadata API and Move remain closed. Removed-empty uses the
existing narrow derived-current role without inventing original-XCF provenance.

The paid recipe separately owns NL/Linear transfer and actual Linear physical
Lab/LCh conversion. It preserves generated Float alpha and finite HDR output.
Exact saturation1 is a stage identity; the full active operation still converts.
Old HDR-native/U8 inputs are refusal references, not unit-source evidence. Strong
F32 and smaller F64 author/fresh producer differences, scalar rounding, profile,
padding and Babl conversion limits remain HOLD; no tolerance or numerical
acceptance follows from public graph equality. New workflows remain UNRUN.

Semi-Flatten, Hue-Chroma and Vibrance now share an explicit checked positive
origin geometry policy: canvas and native drawable bounds must match exactly.
Controls remain candidates only; complete engine CURRENT and staged proof is
required for every supported transaction, including disabled and exact no-op
requests. The source/depth/profile/unit-input/raw-policy and singleton restrictions
remain unchanged. Their full-canvas zero masks may retain multiple valid tiles;
proof includes default words and all physical padding, under existing stored-tile
and actual byte/work limits. Larger removed-empty native `.pcraft` continuation
uses a separate CURRENT role and does not identify a new original XCF source.

All actual caller limits remain binding. Selected bounded services for these
three families and the separate derived float empty CURRENT role use the finite
standard work/deadline policy described below. Structural dimension admission
does not promise that a large current-plus-staged operation fits its allowance. Source Surfaces remain
fully retained; output bands do not imply lazy input or a process memory bound.
This geometry expansion adds no offsets, profiles, Gray, precision, HDR-native
input or filter-stack permission, and changes no numerical acceptance tolerance.
PhotoCraft tests and runtime/native/widget/performance validation remain deferred.

Bounded host work routing for the three pure point families and separate derived
float empty CURRENT uses the existing native_standard 2^40 or wasm_standard 2^36
work ceiling and a 120-second active deadline. A cheap bounded hint requires the
actual owned runtime class and operation or empty-role metadata. It selects an
attempted service budget, never source/current validity, a new operation or a
read-only mutation permission. Rendering/import/edit requests with malformed
family claims still reach their strict endpoint and refuse. Metadata-only queries
keep their existing validation and do not certify renderability. Existing
Box-specific routing and unrelated legacy behavior
are preserved.

Existing filter set/enable/singleton reorder/remove, registered supported layer
metadata, selection, undo/redo and history selection keep one used count, clock
and cancellation token through preparation, old/staged proof, title/view/history
work and publication fences. Bounded inspection, preview and native save wrappers
use the same finite service policy through their existing phases and reply/write
controls. Authentication, RPC/request deadlines, reply limits, source protection
and atomic publication remain mandatory. Prepaid CMS/mask work polls the same
deadline through its Interrupt, rather than waiting for the next work debit.

Explicit caller-supplied controls are unchanged and remain authoritative; lower
limits, cancellation and deadlines cannot be raised or replaced. Source/native
intake already selects standard work before parsing and is not reselected here.
No resource-policy UI, native_large, memory exemption or new Move/compound-metadata
permission is added. Tests, native/widget workflows, timing, memory and numerical
acceptance remain deferred.

The existing Absolute Linear, Channel Mixer, signed Mono Mixer, Color Overlay,
three Component Extract precision and two Alpha Clip precision roles now permit
checked positive full-canvas origin geometry under their existing epochs. Existing
parameter widths/tags/ranges, native/working/result domains, profile/precision,
raw policy and activity rules remain authoritative. Original source activation
must be present, including explicit disabled state. Complete source/current
proof remains mandatory before disabled/hidden shortcuts.

No new command is registered. Existing typed set/enable and supported readonly
layer metadata transactions retain their current/staged and publication fences.
Singleton move and remove, direct compound metadata and additional empty CURRENT
roles remain unavailable for these nine epochs. Their existing controls already
have no fixed-size candidate check. Native continuation means `.pcraft`.

Finite work service requires each exact operation/schema/Replace compatibility
triple, not a six-name alias. Other Mono Original, Component Geometry, residual,
configured and already dynamic families keep their independent policies. Work
routing grants no semantic validity. Existing byte-zero physical mask/default/
padding proof and 16 MiB mask payload cap stay in force; signed zero is not
byte-zero. The newer three-family numeric-zero mask rules are not inherited.
Actual repeated source profile validation and transfer work are charged before
the unchanged conversion. Structural size bounds, finite work and deadlines are
admission ceilings rather than a supported-image-size or completion promise.
All behavioral, numerical, native/widget, timing and performance gates remain
deferred.


For the nine expanded point domains, source qualification now prepares the
same immutable implicit RGB/Gray transfer once per invocation, using the
original cumulative work, deadline and cancellation controls. This eliminates
repeated profile validation; each pixel still pays transfer/decode work and
all existing native and mask checks. No command, caller limit, source profile,
CURRENT rendering rule or publication authority changes. Cancellation remains
a cancellation error, including before output copy. This is an implementation
change with behavioral, native and performance acceptance still deferred.


Seven additional existing point domains accept checked full-canvas origin
geometry: Desaturate, three Saturation variants, precision Value Invert and two
Color Balance variants. Existing typed filter requests and synchronous lanes
remain the interface. Their original profile/precision/working/argument rules
still qualify every request; Color Balance visibility/full-opacity and RGB-Yuv
depth restrictions remain binding. Exact semantic tuples select the existing
finite work service without granting command validity. Caller-lowered controls,
one cumulative ledger/deadline, source protection and publication fences remain.
Larger masks use an explicit actual-grid allowance capped at16MiB, admitted
before full physical/default/padding byte-zero proof. Old8x6 mask behavior is
preserved. No new Move/remove/compound/empty continuation is added. Native
continuation remains `.pcraft`; behavior and performance gates are deferred.


Mixed filtered ordinary roots use a separate readonly `FilteredFlat` handoff.
The current scope is 1..4096 full-canvas origin implicit nonlinear RGB F32/F64
layers, each plain or carrying an ordered stack of existing Semi-Flatten,
Hue-Chroma or Vibrance filters. Globally there are at most 4096 nodes and exactly as many corresponding
masks, and at most 8192 canonical root/mask items. A single
root requires at least two nodes, preserving the strict older singleton route.
There must be at least one stored filter, including when all are disabled. Mixed
sources preserve independent visibility, finite unit effective F32 layer opacity
and explicitly present filter activation. Float-opacity tag33 takes precedence
over legacy byte tag6; both raw records remain exact. Filter opacity stays +1.
This mixed-only source policy leaves strict singleton gates unchanged; hidden,
zero-opacity and disabled roots retain complete native/default/profile/mask and
CURRENT proof. Stored encoding stays separate from root Linear/Linear/Normal
Union policy. No activity-specific resource or publication shortcut is added.

Source intake and native reopen retain actual whole-document native/profile/raw
owners, and strict mixed classification precedes the singleton Point route.
Failure is final. Existing preview, native save/reopen, title and paid paginated
project/filter inspection are connected through this actual class and the same
cumulative controls. Selected/active root metadata is preserved; pixel-selection
editing is unavailable. Existing `xcf.filter.set` and `xcf.filter.enable` now
address an exact existing Semi-Flatten, Hue-Chroma or Vibrance filter on its
actual root and node. The dedicated controls capture one frame's document and
revision, root, actual node index, stack owner/revision and filter; final
association, idle and lock fences still apply.
No generic editable registry fallback is used for this class.

Existing `layer.setProps` supports compound name/visibility/opacity metadata;
the existing captured `layer.renameLayer` wire uses the same paid transaction
with its original selectors, trimmed nonempty name, Rename Layer label, null
reply and original command journal. Rename preserves native all/pixels plus
imported-content locks; compound metadata keeps its existing field-specific
locks and coalescing semantics. No new command id is introduced.

All admitted edits repeat complete OLD/history/STAGED current proof, including
no-op and shared-owner cases, on the same cumulative ledger and deadline.
Actual whole-document copies and reply/journal overlap retain paid native,
source, profile, raw and mask custody. Only a changed target filter stack
revision increments; metadata leaves filter values intact. Final stale-target,
history/channel/view/source/preferences and publication fences remain binding.
Native continuation saves `.pcraft`; this does not add XCF export.

Generic owner adoption, layer movement, filter insertion/removal, masks,
pixel/selection edits and unrelated mutations remain unavailable. Behavioral,
native, numerical, visual and performance acceptance, complete stock support
and the remaining multilayer implementation remain open.


Ordered filters retain their canonical XCF/public GIMP list order for IDs,
inspection and UI indices. Rendering evaluates that list bottom first, while
ordinary roots retain their existing bottom-first document order. All nodes,
including disabled ones, retain complete source/native/profile/mask/current
proof. Each root's actual stored mask tile aggregate is bounded before mapper
transfer/physical scans; earlier logical source scans are separately paid.

The ordered renderer uses an explicitly declared NL Float handoff compatibility
policy between enabled kernels, with additive finite derived-input helpers,
fixed paid working arrays and actual prepared Parts. Initial F64 input and final
Semi output are not implicitly narrowed; disabled nodes pass wide values through
exactly and all-disabled stacks preserve input. GIMP controls establish direction
and matching F32 producer sequences, while F64 sequential differences and unique
graph-format/Float/Babl/FMA/profile equivalence remain unresolved. No numerical
waiver or source HDR import authority is introduced. Native continuation stays
`.pcraft`; filter insertion/removal, layer movement, masks and selection editing
remain outside this increment.


Accounted saving and owned reopen follow the owned-document format route.
They bypass the separate generic IO census and standard native version visitors,
but serialization and loading still call generic Document/XCF validation with
its 1024-node per-stack limit. This corrects the earlier incomplete path trace.
The broader composition bound does not grant native migration beyond this limit.
Model, quota/work and staged-proof bounds remain; runtime acceptance is deferred.

Mixed filtered-flat `xcf.filter.move` accepts the existing two-key wire:
`{"filterId":"123","index":0}`. The ID is canonical decimal text and the
destination is a zero-based integer below both 1024 and the actual stack length.
It moves that actual filter within its own root, retaining its ID, arguments,
mask, raw source custody and undo history. A changed move increments only its
target stack revision; a same-position request still receives full proof and
the upstream revision-headroom check. Both use the existing cumulative controls,
locks, full OLD/history/STAGED validation and final publication fences. Up, Down
and Top controls use the frame's captured target and only valid wire indices.
No new command ID or layer-movement authority is introduced. Ten new regression
groups remain ignored, uncompiled and unrun; PhotoCraft tests remain deferred.
