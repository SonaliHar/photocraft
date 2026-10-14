//! Android tablet gestures that need a little more detail than winit currently exposes.

use egui::{Event, PointerButton, Pos2, RawInput};

/// An Android-host gesture expressed independently from Java/JNI, so its translation stays
/// testable with normal Rust unit tests.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TabletAction {
    /// A two-finger tap should behave exactly like a desktop secondary click.
    SecondaryTap { x: f32, y: f32 },
    /// The S Pen barrel button was held or released while its tip touched the display.
    Barrel { x: f32, y: f32, pressed: bool },
}

/// Rewrites the primary click emitted for a barrel-button touch and appends an atomic secondary
/// click for two fingers. Coordinates are normalized in Java so display density never leaks into
/// the editor's point-based input space.
#[derive(Default)]
pub(crate) struct TabletGestureRouter {
    barrel_down: bool,
    barrel_released: bool,
    touches: Vec<(egui::TouchDeviceId, egui::TouchId, Pos2)>,
    tap_start: Option<f64>,
    tap_center: Option<Pos2>,
    tap_invalid: bool,
    pending_pointer: Vec<Event>,
    pointer_committed: bool,
    multi_touch: bool,
}

impl TabletGestureRouter {
    pub(crate) fn apply(&mut self, raw: &mut RawInput, actions: impl IntoIterator<Item = TabletAction>) {
        let now = raw.time.unwrap_or(0.0);
        let mut touch_tap = None;
        let mut touch_frame = !self.touches.is_empty();
        let mut finished = false;
        for event in &raw.events {
            let Event::Touch { device_id, id, phase, pos, .. } = event else { continue };
            touch_frame = true;
            match phase {
                egui::TouchPhase::Start => {
                    if self.touches.is_empty() {
                        self.tap_start = Some(now);
                        self.tap_center = None;
                        self.tap_invalid = false;
                        self.pointer_committed = false;
                        self.multi_touch = false;
                    }
                    self.touches.push((*device_id, *id, *pos));
                    if let [(_, _, first), (_, _, second)] = self.touches.as_slice() {
                        self.multi_touch = true;
                        self.tap_center = Some(first.lerp(*second, 0.5));
                    } else if self.touches.len() > 2 {
                        self.multi_touch = true;
                        self.tap_invalid = true;
                    }
                }
                egui::TouchPhase::Move => {
                    if let Some((_, _, start)) = self.touches.iter().find(|(device, touch, _)| device == device_id && touch == id)
                        && start.distance(*pos) > 20.0
                    {
                        self.tap_invalid = true;
                    }
                }
                egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                    if *phase == egui::TouchPhase::Cancel {
                        self.tap_invalid = true;
                    }
                    self.touches.retain(|(device, touch, _)| device != device_id || touch != id);
                    if self.touches.is_empty() {
                        finished = true;
                        if !self.tap_invalid && self.tap_start.is_some_and(|start| now - start <= 0.35) {
                            touch_tap = self.tap_center;
                        }
                        self.tap_center = None;
                    }
                }
            }
        }
        // Reserve a short chord window before forwarding the first finger's emulated mouse
        // press. Otherwise a two-finger context click first starts moving the selection.
        if touch_frame && !self.pointer_committed {
            let mut retained = Vec::new();
            for event in std::mem::take(&mut raw.events) {
                if matches!(event, Event::PointerMoved(_) | Event::PointerGone | Event::PointerButton { button: PointerButton::Primary, .. }) {
                    if self.pending_pointer.len() < 256 {
                        self.pending_pointer.push(event);
                    }
                } else {
                    retained.push(event);
                }
            }
            raw.events = retained;
        }
        if self.multi_touch {
            self.pending_pointer.clear();
        } else if !self.pointer_committed && (finished || self.tap_invalid || self.tap_start.is_some_and(|start| now - start >= 0.08)) {
            let mut events = std::mem::take(&mut self.pending_pointer);
            events.append(&mut raw.events);
            raw.events = events;
            self.pointer_committed = true;
        }
        if finished {
            self.tap_start = None;
        }
        let mut secondary_taps = Vec::new();
        let mut barrel_press = None;
        let mut barrel_release = None;
        for action in actions {
            match action {
                TabletAction::SecondaryTap { x, y } => secondary_taps.push((x, y)),
                TabletAction::Barrel { x, y, pressed: true } => {
                    self.barrel_down = true;
                    self.barrel_released = false;
                    barrel_press = Some((x, y));
                }
                TabletAction::Barrel { x, y, pressed: false } => {
                    self.barrel_released = self.barrel_down;
                    self.barrel_down = false;
                    barrel_release = Some((x, y));
                }
            }
        }

        let rewrite_primary = self.barrel_down || self.barrel_released;
        let mut saw_press = false;
        let mut saw_release = false;
        if rewrite_primary {
            for event in &mut raw.events {
                if let Event::PointerButton { button, pressed, .. } = event
                    && *button == PointerButton::Primary
                {
                    *button = PointerButton::Secondary;
                    saw_press |= *pressed;
                    saw_release |= !*pressed;
                }
            }
        }

        if let Some((x, y)) = barrel_press.filter(|_| !saw_press) {
            push_button(raw, x, y, true);
        }
        if let Some((x, y)) = barrel_release.filter(|_| !saw_release) {
            push_button(raw, x, y, false);
        }
        self.barrel_released = false;

        for (x, y) in secondary_taps {
            push_button(raw, x, y, true);
            push_button(raw, x, y, false);
        }
        if let Some(pos) = touch_tap {
            raw.events.push(Event::PointerMoved(pos));
            for pressed in [true, false] {
                raw.events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
            }
        }
    }

    pub(crate) fn waiting_for_chord(&self) -> bool {
        self.tap_start.is_some() && !self.pointer_committed && !self.multi_touch
    }
}

fn push_button(raw: &mut RawInput, x: f32, y: f32, pressed: bool) {
    let Some(pos) = normalized_pos(raw, x, y) else { return };
    raw.events.push(Event::PointerMoved(pos));
    raw.events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers: Default::default() });
}

fn normalized_pos(raw: &RawInput, x: f32, y: f32) -> Option<Pos2> {
    let rect = raw.screen_rect?;
    if !(x.is_finite() && y.is_finite()) {
        return None;
    }
    Some(Pos2::new(rect.left() + x.clamp(0.0, 1.0) * rect.width(), rect.top() + y.clamp(0.0, 1.0) * rect.height()))
}

#[cfg(test)]
mod tests {
    use egui::{Rect, pos2, vec2};

    use super::*;

    fn input(events: Vec<Event>) -> RawInput {
        RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 500.0))), events, ..Default::default() }
    }

    #[test]
    fn staggered_two_finger_tap_never_starts_primary_selection_drag() {
        let mut router = TabletGestureRouter::default();
        let ctx = egui::Context::default();
        let touch = |id, phase, x| Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos: pos2(x, 100.0), force: None };
        let button = |pressed| Event::PointerButton { pos: pos2(100.0, 100.0), button: PointerButton::Primary, pressed, modifiers: Default::default() };
        let frames = [
            vec![touch(1, egui::TouchPhase::Start, 100.0), Event::PointerMoved(pos2(100.0, 100.0)), button(true)],
            vec![touch(2, egui::TouchPhase::Start, 200.0)],
            vec![touch(2, egui::TouchPhase::End, 200.0), touch(1, egui::TouchPhase::End, 100.0), button(false), Event::PointerGone],
        ];
        for (frame, events) in frames.into_iter().enumerate() {
            let mut raw = input(events);
            raw.time = Some(1.0 + frame as f64 * 0.02);
            router.apply(&mut raw, []);
            ctx.begin_pass(raw);
            ctx.input(|i| {
                assert!(!i.pointer.primary_down());
                assert!(!i.pointer.button_pressed(PointerButton::Primary));
                if frame == 2 {
                    assert!(i.pointer.secondary_clicked());
                }
            });
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
        }
    }

    #[test]
    fn single_touch_press_is_forwarded_after_chord_window() {
        let mut router = TabletGestureRouter::default();
        let mut raw = input(vec![
            Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase: egui::TouchPhase::Start, pos: pos2(100.0, 100.0), force: None },
            Event::PointerButton { pos: pos2(100.0, 100.0), button: PointerButton::Primary, pressed: true, modifiers: Default::default() },
        ]);
        raw.time = Some(1.0);
        router.apply(&mut raw, []);
        assert!(router.waiting_for_chord());
        let mut raw = input(vec![]);
        raw.time = Some(1.09);
        router.apply(&mut raw, []);
        assert!(raw.events.iter().any(|event| matches!(event, Event::PointerButton { button: PointerButton::Primary, pressed: true, .. })));
        assert!(!router.waiting_for_chord());
    }

    #[test]
    fn native_two_finger_tap_clicks_but_pinch_does_not() {
        let touch = |id, phase, x| Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(id), phase, pos: pos2(x, 100.0), force: None };
        for moved in [false, true] {
            let mut router = TabletGestureRouter::default();
            let mut raw = input(vec![touch(1, egui::TouchPhase::Start, 100.0), touch(2, egui::TouchPhase::Start, 200.0)]);
            raw.time = Some(1.0);
            router.apply(&mut raw, []);
            let mut events = Vec::new();
            if moved {
                events.push(touch(2, egui::TouchPhase::Move, 250.0));
            }
            events.extend([touch(2, egui::TouchPhase::End, 200.0), touch(1, egui::TouchPhase::End, 100.0)]);
            let mut raw = input(events);
            raw.time = Some(1.1);
            router.apply(&mut raw, []);
            let clicks = raw.events.iter().filter(|event| matches!(event, Event::PointerButton { button: PointerButton::Secondary, .. })).count();
            assert_eq!(clicks, if moved { 0 } else { 2 });
        }
    }

    #[test]
    fn barrel_touch_rewrites_primary_down_and_up_to_secondary() {
        let mut router = TabletGestureRouter::default();
        let down = Event::PointerButton { pos: pos2(20.0, 30.0), button: PointerButton::Primary, pressed: true, modifiers: Default::default() };
        let mut raw = input(vec![down]);
        router.apply(&mut raw, [TabletAction::Barrel { x: 0.02, y: 0.06, pressed: true }]);
        assert!(matches!(raw.events.as_slice(), [Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. }]));

        let up = Event::PointerButton { pos: pos2(20.0, 30.0), button: PointerButton::Primary, pressed: false, modifiers: Default::default() };
        let mut raw = input(vec![up]);
        router.apply(&mut raw, [TabletAction::Barrel { x: 0.02, y: 0.06, pressed: false }]);
        assert!(matches!(raw.events.as_slice(), [Event::PointerButton { button: PointerButton::Secondary, pressed: false, .. }]));
    }

    #[test]
    fn two_finger_tap_adds_a_complete_secondary_click_at_the_normalized_point() {
        let mut raw = input(Vec::new());
        TabletGestureRouter::default().apply(&mut raw, [TabletAction::SecondaryTap { x: 0.25, y: 0.8 }]);
        assert!(
            matches!(raw.events.as_slice(), [Event::PointerMoved(pos), Event::PointerButton { pos: down, button: PointerButton::Secondary, pressed: true, .. }, Event::PointerMoved(release_pos), Event::PointerButton { pos: up, button: PointerButton::Secondary, pressed: false, .. }] if *pos == pos2(250.0, 400.0) && *down == *pos && *release_pos == *pos && *up == *pos)
        );
    }
}
