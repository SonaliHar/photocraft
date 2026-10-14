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
}

impl TabletGestureRouter {
    pub(crate) fn apply(&mut self, raw: &mut RawInput, actions: impl IntoIterator<Item = TabletAction>) {
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
