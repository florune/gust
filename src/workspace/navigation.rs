//! The mouse and the way back, as in GoLand:
//! - Ctrl+click jumps at once, without hovering first;
//! - Alt+drag selects a block of columns, Alt+Shift+click adds a caret;
//! - Back and Forward (Ctrl+Alt+←/→, or the mouse's side buttons) return to
//!   where the caret was before a jump.

use std::path::PathBuf;

use gpui_kit::component::RopeExt as _;
use gpui_kit::component::input::{EditorState, GoToDefinition};
use gpui_kit::*;

use super::Workspace;

actions!(gust, [NavigateBack, NavigateForward]);

pub(super) fn init(cx: &mut App) {
    let workspace = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("ctrl-alt-left", NavigateBack, workspace),
        KeyBinding::new("ctrl-alt-right", NavigateForward, workspace),
    ]);
}

/// How many places Back remembers.
const HISTORY: usize = 100;
/// A press and release this close together is a click, not a drag.
const CLICK_SLOP: f32 = 4.;

/// A file and the caret's line and byte column in it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Place {
    pub path: PathBuf,
    pub row: u32,
    pub col: u32,
}

impl Workspace {
    /// The editor that has the caret: the right pane's when it was used last.
    fn caret_editor(&self) -> Option<(PathBuf, Entity<EditorState>)> {
        if let Some(split) = self.split.as_ref().filter(|split| split.focused) {
            return Some((split.path.clone(), split.editor.clone()));
        }
        let doc = self.active.and_then(|ix| self.docs.get(ix))?;
        Some((doc.path.clone(), doc.editor.clone()))
    }

    pub(super) fn current_place(&self, cx: &App) -> Option<Place> {
        if self.diff_active {
            return None;
        }
        let (path, editor) = self.caret_editor()?;
        let editor = editor.read(cx);
        let position = editor.cursor_position();
        let line = editor.text().slice_line(position.line as usize).to_string();
        // The position counts UTF-16 units; places count bytes.
        let mut units = 0;
        let mut col = line.len();
        for (ix, ch) in line.char_indices() {
            if units >= position.character as usize {
                col = ix;
                break;
            }
            units += ch.len_utf16();
        }
        Some(Place {
            path,
            row: position.line,
            col: col as u32,
        })
    }

    /// Note where the caret is before it jumps somewhere else.
    pub(super) fn remember_place(&mut self, cx: &App) {
        if self.navigating {
            return;
        }
        let Some(place) = self.current_place(cx) else {
            return;
        };
        if self.back.last() != Some(&place) {
            self.back.push(place);
            if self.back.len() > HISTORY {
                self.back.remove(0);
            }
        }
        self.forward.clear();
    }

    fn go_to_place(&mut self, place: Place, window: &mut Window, cx: &mut Context<Self>) {
        self.navigating = true;
        self.open_at(place.path, place.row, place.col, window, cx);
        self.navigating = false;
    }

    pub(super) fn on_navigate_back(&mut self, _: &NavigateBack, window: &mut Window, cx: &mut Context<Self>) {
        let Some(place) = self.back.pop() else {
            return;
        };
        if let Some(here) = self.current_place(cx) {
            self.forward.push(here);
        }
        self.go_to_place(place, window, cx);
    }

    pub(super) fn on_navigate_forward(
        &mut self,
        _: &NavigateForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(place) = self.forward.pop() else {
            return;
        };
        if let Some(here) = self.current_place(cx) {
            self.back.push(here);
        }
        self.go_to_place(place, window, cx);
    }

    /// Runs before anything under the pointer sees the press.
    pub(super) fn navigation_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.button {
            MouseButton::Navigate(NavigationDirection::Back) => {
                cx.stop_propagation();
                self.on_navigate_back(&NavigateBack, window, cx);
            }
            MouseButton::Navigate(NavigationDirection::Forward) => {
                cx.stop_propagation();
                self.on_navigate_forward(&NavigateForward, window, cx);
            }
            MouseButton::Left if !self.remapping.get() => {
                let modifiers = event.modifiers;
                let in_editor = self.editor_at(event.position, cx);
                if modifiers.control
                    && !modifiers.alt
                    && !modifiers.shift
                    && event.click_count == 1
                    && in_editor
                {
                    // Whether the editor jumps by itself is known by the release.
                    self.jumped.set(false);
                    self.ctrl_click = Some(event.position);
                } else if modifiers.alt && !modifiers.control && in_editor {
                    // The editor reads Alt the way VS Code does; swap Shift so
                    // the press means what it means in GoLand.
                    let mut remapped = event.clone();
                    remapped.modifiers.shift = !modifiers.shift;
                    cx.stop_propagation();
                    let remapping = self.remapping.clone();
                    window.defer(cx, move |window, cx| {
                        remapping.set(true);
                        window.dispatch_event(PlatformInput::MouseDown(remapped), cx);
                        remapping.set(false);
                    });
                }
            }
            _ => {}
        }
    }

    /// Any editor shown now, at `position`.
    pub(super) fn editor_at(&self, position: Point<Pixels>, cx: &App) -> bool {
        let shown = self
            .active
            .and_then(|ix| self.docs.get(ix))
            .filter(|_| !self.diff_active)
            .map(|doc| doc.editor.clone())
            .into_iter()
            .chain(self.split.as_ref().map(|split| split.editor.clone()));
        shown
            .into_iter()
            .any(|editor| editor.read(cx).input_bounds().contains(&position))
    }

    /// A Ctrl+click the editor did not act on itself goes to the definition.
    pub(super) fn navigation_mouse_up(
        &mut self,
        event: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            return;
        }
        let Some(down) = self.ctrl_click.take() else {
            return;
        };
        let distance = event.position - down;
        let moved = distance.x.abs() > px(CLICK_SLOP) || distance.y.abs() > px(CLICK_SLOP);
        if moved || !event.modifiers.control || self.jumped.get() {
            return;
        }
        // After the editor's own handling of the release has placed the caret.
        window.defer(cx, |window, cx| {
            window.dispatch_action(Box::new(GoToDefinition), cx);
        });
    }
}
