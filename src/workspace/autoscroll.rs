//! Middle-click autoscroll, as on Windows: click the middle button and the
//! view under it scrolls toward the pointer, faster the farther it is from
//! where the click was. Any click or Esc stops it; holding the button, moving,
//! and letting go stops it too.

use std::time::{Duration, Instant};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use super::Workspace;
use crate::ui::icon;

/// Distance from the anchor in which nothing scrolls, so a resting hand does not.
const DEAD_ZONE: f32 = 14.;
/// Scroll speed per pixel of distance beyond the dead zone, in pixels a frame.
const SPEED: f32 = 0.12;
const FRAME: Duration = Duration::from_millis(16);
/// A middle button held longer than this, or dragged, stops scrolling on release.
const HOLD: Duration = Duration::from_millis(350);

pub(super) struct Autoscroll {
    anchor: Point<Pixels>,
    /// Where the pointer really is. The window's own idea of it moves to the
    /// anchor with every scroll sent there, so it cannot be used.
    pointer: Point<Pixels>,
    started: Instant,
    _task: Task<()>,
}

/// How far to scroll this frame for a pointer `offset` from the anchor, as a
/// wheel delta: positive moves the content down, toward its start.
pub(super) fn velocity(offset: Point<Pixels>) -> Point<Pixels> {
    let axis = |distance: Pixels| {
        let distance = f32::from(distance);
        let beyond = distance.abs() - DEAD_ZONE;
        if beyond <= 0. {
            px(0.)
        } else {
            px(-distance.signum() * beyond * SPEED)
        }
    };
    point(axis(offset.x), axis(offset.y))
}

impl Workspace {
    #[cfg(test)]
    pub(super) fn autoscrolling(&self) -> bool {
        self.autoscroll.is_some()
    }

    fn start_autoscroll(&mut self, anchor: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let task = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(FRAME).await;
                let step = this.update_in(cx, |this, _, _| {
                    let autoscroll = this.autoscroll.as_ref()?;
                    let anchor = autoscroll.anchor;
                    Some((anchor, velocity(autoscroll.pointer - anchor)))
                });
                let Ok(Some((anchor, delta))) = step else {
                    break;
                };
                if delta == Point::default() {
                    continue;
                }
                // A wheel turned at the anchor: whatever scrolls there follows,
                // the editor, a preview, a list, or the terminal alike.
                let event = ScrollEvent {
                    position: anchor,
                    delta: ScrollDelta::Pixels(delta),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                    source: ScrollSource::Pointer,
                };
                let dispatched = cx.update(|window, cx| {
                    window.dispatch_event(event.to_platform_input(), cx);
                });
                if dispatched.is_err() {
                    break;
                }
            }
        });
        self.autoscroll = Some(Autoscroll {
            anchor,
            pointer: anchor,
            started: Instant::now(),
            _task: task,
        });
        cx.notify();
    }

    pub(super) fn stop_autoscroll(&mut self, cx: &mut Context<Self>) {
        if self.autoscroll.take().is_some() {
            cx.notify();
        }
    }

    /// Runs before anything under the pointer sees the press.
    pub(super) fn autoscroll_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.autoscroll.is_some() {
            // The click that ends autoscroll does nothing else.
            self.stop_autoscroll(cx);
            cx.stop_propagation();
        } else if event.button == MouseButton::Middle {
            self.start_autoscroll(event.position, window, cx);
            cx.stop_propagation();
        }
    }

    pub(super) fn autoscroll_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if let Some(autoscroll) = &mut self.autoscroll {
            autoscroll.pointer = event.position;
        }
    }

    pub(super) fn autoscroll_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(autoscroll) = &self.autoscroll else {
            return;
        };
        if event.button != MouseButton::Middle {
            return;
        }
        let moved = velocity(event.position - autoscroll.anchor) != Point::default();
        if moved || autoscroll.started.elapsed() > HOLD {
            self.stop_autoscroll(cx);
        }
    }

    pub(super) fn autoscroll_key_down(
        &mut self,
        event: &KeyDownEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.autoscroll.is_some() {
            self.stop_autoscroll(cx);
            if event.keystroke.key == "escape" {
                cx.stop_propagation();
            }
        }
    }

    /// The marker at the anchor, and the cursor that says which way things go.
    pub(super) fn render_autoscroll(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let autoscroll = self.autoscroll.as_ref()?;
        let theme = cx.theme();
        let size = px(30.);
        Some(
            div()
                .absolute()
                .inset_0()
                .cursor(CursorStyle::ResizeUpDown)
                .child(
                    div()
                        .absolute()
                        .left(autoscroll.anchor.x - size / 2.)
                        .top(autoscroll.anchor.y - size / 2.)
                        .size(size)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.popover)
                        .border_1()
                        .border_color(theme.border)
                        .shadow_md()
                        .text_color(theme.muted_foreground)
                        .child(icon("move").size(px(16.))),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::velocity;
    use gpui_kit::{point, px};

    #[test]
    fn nothing_scrolls_near_the_anchor_and_farther_is_faster() {
        assert_eq!(velocity(point(px(5.), px(-10.))), point(px(0.), px(0.)));
        let slow = velocity(point(px(0.), px(64.))).y;
        let fast = velocity(point(px(0.), px(214.))).y;
        // Below the anchor the content moves up, as a wheel turned down does.
        assert!(slow < px(0.) && fast < slow);
        assert!(velocity(point(px(0.), px(-64.))).y > px(0.));
        assert!(velocity(point(px(100.), px(0.))).x < px(0.));
    }
}
