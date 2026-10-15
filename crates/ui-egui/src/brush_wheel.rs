//! Optional plain-wheel stroke sizing. Consume raw wheel events so one notch produces one
//! command and egui's smoothed tail cannot pan the canvas or change the popup twice.

use egui::{Context, Event, Id, MouseWheelUnit, Vec2};

pub(crate) fn consume(ctx: &Context, owner: Id, active: bool, invert: bool) -> Option<i32> {
    let residual_id = owner.with("brush-wheel-residual");
    let captured_id = owner.with("brush-wheel-captured");
    if !active {
        ctx.data_mut(|d| {
            d.remove::<f32>(residual_id);
            d.remove::<bool>(captured_id);
        });
        return None;
    }
    let mut residual = ctx.data(|d| d.get_temp::<f32>(residual_id)).unwrap_or(0.0);
    let mut captured = ctx.data(|d| d.get_temp::<bool>(captured_id)).unwrap_or(false);
    let per_line = ctx.options(|o| o.input_options.line_scroll_speed).max(1.0);
    let mut steps = 0_i32;
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let Event::MouseWheel { unit, delta, modifiers, .. } = event else { return true };
            captured = modifiers.is_none() && delta.y.is_finite() && delta.x.is_finite() && delta.y.abs() > delta.x.abs();
            if !captured {
                residual = 0.0;
                return true;
            }
            let notches = match unit {
                MouseWheelUnit::Point => delta.y / per_line,
                MouseWheelUnit::Line | MouseWheelUnit::Page => delta.y,
            };
            residual += notches.clamp(-32.0, 32.0);
            let whole = residual.trunc() as i32;
            residual -= whole as f32;
            steps = steps.saturating_add(whole).clamp(-32, 32);
            false
        });
        if captured {
            input.smooth_scroll_delta = Vec2::ZERO;
        }
    });
    ctx.data_mut(|d| {
        d.insert_temp(residual_id, residual);
        d.insert_temp(captured_id, captured);
    });
    captured.then_some(if invert { -steps.clamp(-32, 32) } else { steps.clamp(-32, 32) })
}
