//! Mouse wheel and trackpad navigation over the canvas (#293, #635, #5363):
//!
//! - The wheel scrolls up and down; ⇧ or ⌘ (Mac) + wheel scrolls sideways.
//! - Ctrl + a mouse-wheel notch zooms around the pointer, 10% per notch, the frame it arrives.
//!   Windows reports Ctrl as the command key, which egui would otherwise fold into a sideways
//!   scroll; a whole line-notch is taken back out of that pan (#5363).
//! - ⌥/Alt + wheel zooms the same way.
//! - Preferences › General › Zoom with Scroll Wheel swaps the plain wheel and ⌥/Alt.
//!   Ctrl + a notch still zooms.
//! - A trackpad pinch zooms around the pointer, unless Preferences › Enhanced Controls ›
//!   Zoom with Trackpad Pinch is off (then a pinch does nothing, and Windows' Ctrl + wheel
//!   fractions scroll sideways like the wheel, because the fold below is skipped).
//!
//! [`configure`] makes egui fold ⇧ and ⌘/Ctrl + wheel into a sideways scroll, so only a real
//! pinch (egui's `Event::Zoom`, or a touch screen) reaches `zoom_delta`. macOS and browsers
//! deliver a pinch as its own event, but Windows hands a precision touchpad pinch to apps
//! as Ctrl + wheel in fractions of a notch (also on some Linux Wayland systems):
//! [`fold_legacy_pinch`] turns those back into a pinch before egui sees them. A whole
//! mouse-wheel notch with Ctrl held is not a pinch: [`read`] zooms by ×1.1 instead of
//! scrolling sideways.
//!
//! The target behaviour is exact ×1.1 steps, one per notch, with no easing in between
//! (measured frame by frame on a screen recording). egui smooths a wheel notch over several
//! frames, so zooming reads the wheel events themselves and ignores that smoothed tail; panning
//! keeps the smoothed delta. The Alt state is taken from the wheel events and kept until the next
//! wheel event: releasing Alt while the tail is still settling must not turn it into a pan.

use egui::{Context, Event, Id, Modifiers, MouseWheelUnit, RawInput, Vec2};

/// Zoom factor of one wheel notch: every step is exactly ×1.1.
pub const NOTCH: f32 = 1.1;

/// A Ctrl + wheel event this soon (seconds) after a pinch step still belongs to the pinch, even
/// when it happens to be a whole notch.
const PINCH_GAP: f64 = 0.25;

/// What the wheel does to the view this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wheel {
    /// Multiply the zoom by this factor around the pointer.
    Zoom(f32),
    /// Move the image by this many screen points (egui's content direction).
    Pan(Vec2),
}

/// Inputs of one frame's wheel gesture.
#[derive(Clone, Copy, Debug)]
pub struct Input {
    /// Smoothed scroll delta (points, content direction); what panning uses.
    pub scroll: Vec2,
    /// This frame's wheel events for the gesture in progress, unsmoothed (points, content
    /// direction); what zooming uses.
    pub raw: Vec2,
    /// egui's pinch zoom factor (1 = none).
    pub zoom_delta: f32,
    /// ⌥/Alt was held for the wheel gesture in progress.
    pub alt: bool,
    /// Preferences › General › Zoom with Scroll Wheel.
    pub zoom_with_wheel: bool,
    /// Preferences › Enhanced Controls › Zoom with Trackpad Pinch.
    pub pinch_zooms: bool,
    /// Points per wheel notch (egui's `line_scroll_speed`).
    pub notch: f32,
    /// Ctrl + a whole mouse-wheel notch (#5363). The smoothed tail must not pan.
    pub ctrl_zoom: bool,
}

/// The view change for one frame of wheel input, if any.
pub fn classify(i: Input) -> Option<Wheel> {
    if i.pinch_zooms && i.zoom_delta.is_finite() && i.zoom_delta > 0.0 && i.zoom_delta != 1.0 {
        return Some(Wheel::Zoom(i.zoom_delta));
    }
    // One notch (`notch` points) is one ×1.1 step.
    let step = |dy: f32| {
        let notch = if i.notch.is_finite() && i.notch > 0.0 { i.notch } else { 40.0 };
        let f = NOTCH.powf(dy / notch);
        (f.is_finite() && f > 0.0 && f != 1.0).then_some(Wheel::Zoom(f))
    };
    // Ctrl + a whole mouse notch zooms on the frame it arrives. Its smoothed tail is not a pan.
    if i.ctrl_zoom {
        return step(i.raw.y + i.raw.x);
    }
    match (i.alt, i.zoom_with_wheel) {
        // egui folds a scroll with Alt held into y. The smoothed tail of a notch is ignored.
        (true, false) => step(i.raw.y + i.raw.x),
        // ⇧ and Mac ⌘ fold the wheel into x: those still scroll sideways.
        (false, true) if i.raw.y != 0.0 || i.scroll.y != 0.0 => step(i.raw.y),
        _ if !(i.scroll.x.is_finite() && i.scroll.y.is_finite()) || i.scroll == Vec2::ZERO => None,
        _ => Some(Wheel::Pan(i.scroll)),
    }
}

/// Set egui's wheel modifiers (once, at setup): ⇧ and ⌘/Ctrl + wheel scroll sideways.
/// A Ctrl mouse-wheel notch is turned back into a zoom in [`read`] (#5363). Mac ⌘ stays a
/// sideways pan, because it sets `mac_cmd` and not `ctrl`.
pub fn configure(ctx: &Context) {
    ctx.options_mut(|o| {
        o.input_options.zoom_modifier = Modifiers::NONE;
        o.input_options.horizontal_scroll_modifier = Modifiers::SHIFT | Modifiers::COMMAND;
    });
}

/// Turn a Windows or Linux touchpad pinch, which arrives as Ctrl + wheel in fractions of a notch or points, into
/// pinch events (`Event::Zoom`) at egui's Ctrl-scroll zoom speed (`notch` points per line,
/// `speed` per point). Ctrl + wheel in whole notches (a mouse wheel) is left as a wheel event
/// ([`read`] zooms those by ×1.1), unless it comes within [`PINCH_GAP`] of a pinch step. `last`
/// is the time of the previous pinch step; returns the time of the latest one.
pub fn legacy_pinch_to_zoom(events: &mut [Event], now: f64, last: Option<f64>, notch: f32, speed: f32) -> Option<f64> {
    let ctrl_wheel = |e: &Event| match e {
        Event::MouseWheel { unit: MouseWheelUnit::Line, delta, modifiers, .. } if modifiers.ctrl => Some((*delta, false)),
        // Wayland precision touchpads can use point units. These are already actual
        // screen points, not line notches; normalize before the exponential below.
        Event::MouseWheel { unit: MouseWheelUnit::Point, delta, modifiers, .. } if modifiers.ctrl => Some((*delta / notch.max(1.0), true)),
        _ => None,
    };
    let fraction = |d: Vec2| d.x.fract() != 0.0 || d.y.fract() != 0.0;
    let pinching =
        last.is_some_and(|t| (0.0..PINCH_GAP).contains(&(now - t))) || events.iter().filter_map(ctrl_wheel).any(|(d, high_res)| high_res || fraction(d));
    if !pinching {
        return last;
    }
    let mut latest = last;
    for e in events.iter_mut() {
        if let Some((d, _)) = ctrl_wheel(e) {
            let f = (speed * notch * (d.x + d.y)).exp();
            *e = Event::Zoom(if f.is_finite() && f > 0.0 { f } else { 1.0 });
            latest = Some(now);
        }
    }
    latest
}

fn pinch_id() -> Id {
    Id::new("pc-wheel-legacy-pinch")
}

/// [`legacy_pinch_to_zoom`] for one frame's raw input (`raw_input_hook`, Windows and Linux).
pub fn fold_legacy_pinch(ctx: &Context, raw: &mut RawInput) {
    // Native gesture events already carry their own magnification. Some backends
    // also emit a Ctrl-wheel event in the same frame: folding that wheel event
    // would apply the same pinch twice. End any legacy continuation as well,
    // so a later real Ctrl+wheel notch isn't treated as part of that pinch.
    if raw.events.iter().any(|event| matches!(event, Event::Zoom(_))) {
        ctx.data_mut(|d| d.remove::<f64>(pinch_id()));
        return;
    }
    let (notch, speed) = ctx.options(|o| (o.input_options.line_scroll_speed, o.input_options.scroll_zoom_speed));
    let now = raw.time.unwrap_or_else(|| ctx.input(|i| i.time));
    let last = ctx.data(|d| d.get_temp::<f64>(pinch_id()));
    let latest = legacy_pinch_to_zoom(&mut raw.events, now, last, notch, speed);
    if latest != last
        && let Some(t) = latest
    {
        ctx.data_mut(|d| d.insert_temp(pinch_id(), t));
    }
}

fn alt_id() -> Id {
    Id::new("pc-wheel-alt")
}

fn ctrl_id() -> Id {
    Id::new("pc-wheel-ctrl-zoom")
}

/// Ctrl + a whole mouse-wheel notch (line units, no fraction). Windows and Linux set `command`
/// as well as `ctrl`; a Mac ⌘ scroll sets `mac_cmd` and is a sideways pan. Fractions are a
/// touchpad pinch ([`fold_legacy_pinch`]), or a sideways scroll when that preference is off.
fn ctrl_mouse_zoom(unit: MouseWheelUnit, delta: Vec2, m: Modifiers) -> bool {
    unit == MouseWheelUnit::Line
        && m.ctrl
        && !m.mac_cmd
        && !m.alt
        && !m.shift
        && delta.x.fract() == 0.0
        && delta.y.fract() == 0.0
        && (delta.x != 0.0 || delta.y != 0.0)
}

/// Read this frame's wheel input. Call it every frame (hovered or not) so the Alt state of the
/// gesture follows the latest wheel event.
pub fn read(ctx: &Context, zoom_with_wheel: bool, pinch_zooms: bool) -> Option<Wheel> {
    let notch = ctx.options(|o| o.input_options.line_scroll_speed);
    let (scroll, zoom_delta, latest_alt, latest_ctrl, wheel, page) = ctx.input(|i| {
        let latest = i.events.iter().rev().find_map(|e| match e {
            Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, *modifiers)),
            _ => None,
        });
        let latest_alt = latest.map(|(_, _, m)| m.alt && !m.command && !m.ctrl);
        let latest_ctrl = latest.map(|(unit, delta, m)| ctrl_mouse_zoom(unit, delta, m));
        let wheel: Vec<(MouseWheelUnit, Vec2, Modifiers)> = i
            .events
            .iter()
            .filter_map(|e| match e {
                Event::MouseWheel { unit, delta, modifiers, .. } => Some((*unit, *delta, *modifiers)),
                _ => None,
            })
            .collect();
        (i.smooth_scroll_delta, i.zoom_delta(), latest_alt, latest_ctrl, wheel, i.content_rect().height())
    });
    let alt = match latest_alt {
        Some(a) => {
            ctx.data_mut(|d| d.insert_temp(alt_id(), a));
            a
        }
        None => ctx.data(|d| d.get_temp::<bool>(alt_id())).unwrap_or(false),
    };
    // Held until the next wheel event, so the smoothed tail of a Ctrl notch does not pan.
    let ctrl_zoom = match latest_ctrl {
        Some(z) => {
            ctx.data_mut(|d| d.insert_temp(ctrl_id(), z));
            z
        }
        None => ctx.data(|d| d.get_temp::<bool>(ctrl_id())).unwrap_or(false),
    };
    // The wheel events that belong to the gesture in progress. ⇧ and Mac ⌘ scroll sideways.
    // A whole Ctrl notch zooms (#5363); a fractional Ctrl wheel is a pinch, or a sideways pan
    // when pinch zoom is off.
    let raw = wheel
        .iter()
        .filter(|(unit, delta, m)| {
            if ctrl_zoom {
                ctrl_mouse_zoom(*unit, *delta, *m)
            } else if alt {
                m.alt && !m.command && !m.ctrl
            } else {
                !m.alt && !m.shift && !m.command && !m.ctrl
            }
        })
        .fold(Vec2::ZERO, |sum, (unit, delta, _)| {
            sum + *delta
                * match unit {
                    MouseWheelUnit::Point => 1.0,
                    MouseWheelUnit::Line => notch,
                    MouseWheelUnit::Page => page,
                }
        });
    classify(Input { scroll, raw, zoom_delta, alt, zoom_with_wheel, pinch_zooms, notch, ctrl_zoom })
}

#[cfg(test)]
mod tests {
    use egui::TouchPhase;

    use super::*;

    fn input(scroll: Vec2, alt: bool) -> Input {
        Input { scroll, raw: scroll, zoom_delta: 1.0, alt, zoom_with_wheel: false, pinch_zooms: true, notch: 40.0, ctrl_zoom: false }
    }

    fn wheel(delta: Vec2, modifiers: Modifiers) -> Event {
        Event::MouseWheel { unit: MouseWheelUnit::Line, delta, phase: TouchPhase::Move, modifiers }
    }

    const CTRL: Modifiers = Modifiers { ctrl: true, command: true, ..Modifiers::NONE };
    const CMD: Modifiers = Modifiers { mac_cmd: true, command: true, ..Modifiers::NONE };

    /// Feed `events` through an egui context set up like the app's (`legacy`: the Windows pinch
    /// fold on) and add up what the wheel did to the view over the following second.
    fn gesture(frames: Vec<Vec<Event>>, zoom_with_wheel: bool, legacy: bool) -> (Vec2, f32) {
        let ctx = Context::default();
        configure(&ctx);
        let (mut pan, mut zoom) = (Vec2::ZERO, 1.0);
        let mut frames = frames.into_iter();
        for n in 0..60 {
            let mut raw =
                RawInput { time: Some(f64::from(n) / 60.0), predicted_dt: 1.0 / 60.0, events: frames.next().unwrap_or_default(), ..Default::default() };
            if legacy {
                fold_legacy_pinch(&ctx, &mut raw);
            }
            let mut out = ctx.run_ui(raw, |ui| match read(ui.ctx(), zoom_with_wheel, true) {
                Some(Wheel::Pan(d)) => pan += d,
                Some(Wheel::Zoom(f)) => zoom *= f,
                None => {}
            });
            out.textures_delta.clear();
        }
        (pan, zoom)
    }

    fn near(a: Vec2, b: Vec2) -> bool {
        (a - b).length() < 0.01
    }

    #[test]
    fn plain_scroll_pans() {
        assert_eq!(classify(input(Vec2::new(3.0, -40.0), false)), Some(Wheel::Pan(Vec2::new(3.0, -40.0))));
        assert_eq!(classify(input(Vec2::ZERO, false)), None);
    }

    #[test]
    fn alt_scroll_zooms_ten_percent_per_notch() {
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, 40.0), true)) else { panic!("zoom") };
        assert!((f - 1.1).abs() < 1e-5, "{f}");
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, -40.0), true)) else { panic!("zoom") };
        assert!((f - 1.0 / 1.1).abs() < 1e-5, "{f}");
        // Two notches in one frame are two steps, x1.1 each.
        let Some(Wheel::Zoom(f)) = classify(input(Vec2::new(0.0, 80.0), true)) else { panic!("zoom") };
        assert!((f - 1.21).abs() < 1e-4, "{f}");
    }

    #[test]
    fn a_notch_zooms_the_frame_it_arrives_and_its_smoothed_tail_does_nothing() {
        // egui keeps feeding a smoothed delta for several frames after the wheel event. Zooming
        // must not follow it (that would ease the zoom), nor turn it into a pan.
        let tail = Input { raw: Vec2::ZERO, ..input(Vec2::new(0.0, 12.0), true) };
        assert_eq!(classify(tail), None);
        let tail = Input { raw: Vec2::ZERO, zoom_with_wheel: true, ..input(Vec2::new(0.0, 12.0), false) };
        assert_eq!(classify(tail), None);
        // Plain scrolling without the preference still pans from the smoothed delta.
        let pan = Input { raw: Vec2::ZERO, ..input(Vec2::new(0.0, 12.0), false) };
        assert_eq!(classify(pan), Some(Wheel::Pan(Vec2::new(0.0, 12.0))));
    }

    #[test]
    fn pinch_zooms() {
        let i = Input { zoom_delta: 1.2, ..input(Vec2::ZERO, false) };
        assert_eq!(classify(i), Some(Wheel::Zoom(1.2)));
        // The pinch wins over Alt.
        let i = Input { zoom_delta: 0.8, ..input(Vec2::new(0.0, 40.0), true) };
        assert_eq!(classify(i), Some(Wheel::Zoom(0.8)));
    }

    /// Preferences › Enhanced Controls › Zoom with Trackpad Pinch off: the pinch is ignored, and
    /// a scroll in the same frame pans as it would without a pinch.
    #[test]
    fn the_pinch_preference_switches_pinch_zooming() {
        let i = Input { zoom_delta: 1.2, pinch_zooms: false, ..input(Vec2::ZERO, false) };
        assert_eq!(classify(i), None);
        let i = Input { zoom_delta: 1.2, pinch_zooms: false, ..input(Vec2::new(0.0, 12.0), false) };
        assert_eq!(classify(i), Some(Wheel::Pan(Vec2::new(0.0, 12.0))));
        // On (the default) the pinch still zooms.
        let i = Input { zoom_delta: 1.2, pinch_zooms: true, ..input(Vec2::ZERO, false) };
        assert_eq!(classify(i), Some(Wheel::Zoom(1.2)));
    }

    /// The Windows pinch fold follows the preference too: off, the Ctrl + wheel fractions stay
    /// wheel events (which egui folds into a sideways scroll).
    #[test]
    fn the_windows_pinch_fold_follows_the_preference() {
        use eframe::App as _;
        if !cfg!(target_os = "windows") {
            return;
        }
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = Context::default();
        let event = || Event::MouseWheel {
            unit: MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 0.25),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers { ctrl: true, command: true, ..Modifiers::NONE },
        };
        let mut raw = RawInput { time: Some(1.0), events: vec![event()], ..Default::default() };
        app.raw_input_hook(&ctx, &mut raw);
        assert!(matches!(raw.events[0], Event::Zoom(_)), "the fraction is a pinch: {:?}", raw.events[0]);
        app.run("prefs.set", serde_json::json!({"path": "enhancedControls.zoomWithTrackpadPinch", "value": false})).unwrap();
        let mut raw = RawInput { time: Some(2.0), events: vec![event()], ..Default::default() };
        app.raw_input_hook(&ctx, &mut raw);
        assert!(matches!(raw.events[0], Event::MouseWheel { .. }), "off: it stays a wheel event: {:?}", raw.events[0]);
    }

    #[test]
    fn zoom_with_scroll_wheel_preference_swaps_wheel_and_alt_wheel() {
        let pref = |scroll, alt| Input { zoom_with_wheel: true, ..input(scroll, alt) };
        let Some(Wheel::Zoom(f)) = classify(pref(Vec2::new(0.0, 40.0), false)) else { panic!("zoom") };
        assert!((f - 1.1).abs() < 1e-5, "{f}");
        assert_eq!(classify(pref(Vec2::new(0.0, 40.0), true)), Some(Wheel::Pan(Vec2::new(0.0, 40.0))), "⌥ + wheel scrolls");
        assert_eq!(classify(pref(Vec2::new(40.0, 0.0), false)), Some(Wheel::Pan(Vec2::new(40.0, 0.0))), "sideways still scrolls");
    }

    #[test]
    fn each_modifier_maps_like_photoshop() {
        let notch = Vec2::new(0.0, 1.0);
        let one = |m: Modifiers| gesture(vec![vec![wheel(notch, m)]], false, false);
        let (pan, zoom) = one(Modifiers::NONE);
        assert!(near(pan, Vec2::new(0.0, 40.0)) && zoom == 1.0, "wheel scrolls vertically: {pan:?} {zoom}");
        for m in [Modifiers::SHIFT, CMD] {
            let (pan, zoom) = one(m);
            assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "{m:?} + wheel scrolls sideways: {pan:?} {zoom}");
        }
        // Windows reports Ctrl as command+ctrl. Either form zooms; the smoothed tail does not pan.
        for m in [CTRL, Modifiers::CTRL] {
            let (pan, zoom) = one(m);
            assert!(pan == Vec2::ZERO && (zoom - 1.1).abs() < 1e-4, "{m:?} + wheel zooms: {pan:?} {zoom}");
        }
        let (pan, zoom) = one(Modifiers::ALT);
        assert!(pan == Vec2::ZERO && (zoom - 1.1).abs() < 1e-4, "⌥ + wheel zooms 10%, at once: {pan:?} {zoom}");
        // Zoom with Scroll Wheel on: swapped.
        let (pan, zoom) = gesture(vec![vec![wheel(notch, Modifiers::ALT)]], true, false);
        assert!(near(pan, Vec2::new(0.0, 40.0)) && zoom == 1.0, "{pan:?} {zoom}");
        let (pan, zoom) = gesture(vec![vec![wheel(notch, Modifiers::NONE)]], true, false);
        assert!(pan == Vec2::ZERO && (zoom - 1.1).abs() < 1e-4, "{pan:?} {zoom}");
        let (pan, zoom) = gesture(vec![vec![wheel(notch, CTRL)]], true, false);
        assert!(pan == Vec2::ZERO && (zoom - 1.1).abs() < 1e-4, "Ctrl + wheel still zooms: {pan:?} {zoom}");
        let (pan, zoom) = gesture(vec![vec![wheel(notch, Modifiers::SHIFT)]], true, false);
        assert!(near(pan, Vec2::new(40.0, 0.0)) && zoom == 1.0, "Shift + wheel still scrolls sideways: {pan:?} {zoom}");
    }

    #[test]
    fn pinch_zooms_and_ctrl_wheel_pans() {
        // macOS trackpad and browsers: the pinch is its own event.
        let (pan, zoom) = gesture(vec![vec![Event::Zoom(1.1)], vec![Event::Zoom(1.1)]], false, false);
        assert!(pan == Vec2::ZERO && (zoom - 1.21).abs() < 1e-4, "{pan:?} {zoom}");
        // Windows: a touchpad pinch is Ctrl + wheel in fractions of a notch, a mouse wheel whole notches.
        let (pan, zoom) = gesture(vec![vec![wheel(Vec2::new(0.0, 0.25), CTRL)], vec![wheel(Vec2::new(0.0, 0.5), CTRL)]], false, true);
        let speed = egui::InputOptions::default().scroll_zoom_speed;
        assert!(pan == Vec2::ZERO && (zoom - (speed * 40.0 * 0.75).exp()).abs() < 1e-4, "the pinch zooms: {pan:?} {zoom}");
        let (pan, zoom) = gesture(vec![vec![wheel(Vec2::new(0.0, 1.0), CTRL)]], false, true);
        assert!(pan == Vec2::ZERO && (zoom - 1.1).abs() < 1e-4, "Ctrl + mouse wheel zooms, and its tail does not pan: {pan:?} {zoom}");
        // Pinch preference off: the fraction is not folded, so it stays the sideways scroll.
        let (pan, zoom) = gesture(vec![vec![wheel(Vec2::new(0.0, 0.25), CTRL)]], false, false);
        assert!(zoom == 1.0 && near(pan, Vec2::new(10.0, 0.0)), "a Ctrl fraction without the pinch fold pans sideways: {pan:?} {zoom}");
    }

    /// #5363: egui has already folded Ctrl into a sideways delta. The notch still zooms, and the
    /// smoothed sideways tail does not pan.
    #[test]
    fn ctrl_notch_zooms_and_ignores_the_sideways_tail() {
        let sideways = Vec2::new(40.0, 0.0);
        let i = Input { raw: Vec2::new(0.0, 40.0), ctrl_zoom: true, ..input(sideways, false) };
        let Some(Wheel::Zoom(f)) = classify(i) else { panic!("zoom") };
        assert!((f - 1.1).abs() < 1e-5, "{f}");
        let tail = Input { raw: Vec2::ZERO, ctrl_zoom: true, ..input(sideways, false) };
        assert_eq!(classify(tail), None, "the smoothed sideways tail is not a pan");
        let shift = classify(input(sideways, false));
        assert_eq!(shift, Some(Wheel::Pan(sideways)), "Shift / ⌘ sideways pan is unchanged");
    }

    #[test]
    fn linux_wayland_point_unit_pinch_keeps_mouse_notches_for_sideways_scroll() {
        // Some Wayland compositors report fractional Ctrl-wheel pinch steps in points
        // rather than line units. The delta must not be multiplied by notch twice.
        let mut point = [Event::MouseWheel { unit: MouseWheelUnit::Point, delta: Vec2::new(0.0, 8.0), phase: TouchPhase::Move, modifiers: CTRL }];
        assert_eq!(legacy_pinch_to_zoom(&mut point, 1.0, None, 40.0, 0.005), Some(1.0));
        assert!(matches!(point[0], Event::Zoom(f) if (f - (8.0f32 * 0.005).exp()).abs() < 1e-5));
        let mut wheel_only = [wheel(Vec2::new(0.0, 1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut wheel_only, 2.0, Some(1.0), 40.0, 0.005), Some(1.0));
        assert!(matches!(wheel_only[0], Event::MouseWheel { .. }), "whole mouse notch is not a pinch");
    }

    #[test]
    fn native_pinch_is_not_zoomed_twice_when_ctrl_wheel_arrives_in_the_same_frame() {
        let (_, zoom) = gesture(vec![vec![Event::Zoom(1.2), wheel(Vec2::new(0.0, 0.25), CTRL)]], false, true);
        assert!((zoom - 1.2).abs() < 1e-4, "native zoom must win over fallback: {zoom}");
    }

    #[test]
    fn native_pinch_resets_legacy_gesture_continuation() {
        let ctx = Context::default();
        let mut legacy = RawInput { time: Some(1.0), events: vec![wheel(Vec2::new(0.0, 0.25), CTRL)], ..Default::default() };
        fold_legacy_pinch(&ctx, &mut legacy);
        assert!(matches!(legacy.events[0], Event::Zoom(_)));

        // Native magnification takes over before the synthetic gesture times out.
        let mut native = RawInput { time: Some(1.1), events: vec![Event::Zoom(1.1), wheel(Vec2::new(0.0, 0.25), CTRL)], ..Default::default() };
        fold_legacy_pinch(&ctx, &mut native);
        assert!(matches!(native.events[0], Event::Zoom(f) if (f - 1.1).abs() < 1e-5));
        assert!(matches!(native.events[1], Event::MouseWheel { .. }));

        // A notched mouse scroll must no longer inherit the previous pinch.
        let mut notch = RawInput { time: Some(1.12), events: vec![wheel(Vec2::new(0.0, 1.0), CTRL)], ..Default::default() };
        fold_legacy_pinch(&ctx, &mut notch);
        assert!(matches!(notch.events[0], Event::MouseWheel { .. }));
    }

    #[test]
    fn a_whole_notch_inside_a_pinch_stays_a_pinch() {
        let mut ev = [wheel(Vec2::new(0.0, 0.5), CTRL)];
        let t = legacy_pinch_to_zoom(&mut ev, 1.0, None, 40.0, 0.005);
        assert_eq!(t, Some(1.0));
        assert!(matches!(ev[0], Event::Zoom(f) if (f - 0.1f32.exp()).abs() < 1e-5), "{ev:?}");
        // The next step lands on a whole notch: still the pinch.
        let mut ev = [wheel(Vec2::new(0.0, -1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 1.1, t, 40.0, 0.005), Some(1.1));
        assert!(matches!(ev[0], Event::Zoom(f) if f < 1.0), "{ev:?}");
        // Long after: a mouse wheel notch, untouched.
        let mut ev = [wheel(Vec2::new(0.0, -1.0), CTRL)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 2.0, Some(1.1), 40.0, 0.005), Some(1.1));
        assert!(matches!(ev[0], Event::MouseWheel { .. }));
        // Without Ctrl nothing is a pinch.
        let mut ev = [wheel(Vec2::new(0.0, 0.5), Modifiers::NONE)];
        assert_eq!(legacy_pinch_to_zoom(&mut ev, 2.0, None, 40.0, 0.005), None);
        assert!(matches!(ev[0], Event::MouseWheel { .. }));
    }

    #[test]
    fn hostile_numbers_never_produce_a_bad_zoom() {
        for s in [f32::NAN, f32::INFINITY, -f32::INFINITY, 1e30, -1e30] {
            for alt in [false, true] {
                for notch in [0.0, -1.0, f32::NAN, 40.0] {
                    for zd in [f32::NAN, 0.0, -1.0, f32::INFINITY, 1.0] {
                        let i = Input {
                            scroll: Vec2::new(0.0, s),
                            raw: Vec2::new(0.0, s),
                            zoom_delta: zd,
                            alt,
                            zoom_with_wheel: !alt,
                            pinch_zooms: true,
                            notch,
                            ctrl_zoom: false,
                        };
                        if let Some(Wheel::Zoom(f)) = classify(i) {
                            assert!(f.is_finite() && f > 0.0, "{f} from {i:?}");
                        }
                    }
                    let mut ev = [wheel(Vec2::new(s, 0.5), CTRL)];
                    let _ = legacy_pinch_to_zoom(&mut ev, f64::NAN, Some(f64::NAN), notch, 0.005);
                    let _ = legacy_pinch_to_zoom(&mut ev, 0.0, None, notch, f32::NAN);
                    if let Event::Zoom(f) = ev[0] {
                        assert!(f.is_finite() && f > 0.0, "{f}");
                    }
                }
            }
        }
    }
}
