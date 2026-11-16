//! Ctrl+W selects the next larger piece of code, Ctrl+Shift+W goes back a
//! step, as in GoLand.

use std::ops::Range;

use gpui_kit::component::input::EditorState;
use gpui_kit::*;

use super::Workspace;
use crate::expand;

actions!(gust, [ExtendSelection, ShrinkSelection]);

pub(super) fn init(cx: &mut App) {
    let editor = Some("Input && mode == editor");
    cx.bind_keys([
        KeyBinding::new("ctrl-w", ExtendSelection, editor),
        KeyBinding::new("ctrl-shift-w", ShrinkSelection, editor),
    ]);
}

/// The selections Ctrl+W went through, to step back through with Ctrl+Shift+W.
#[derive(Default)]
pub(super) struct Expansions {
    editor: Option<EntityId>,
    /// What Ctrl+W selected last; any other selection starts over.
    current: Option<Range<usize>>,
    before: Vec<Range<usize>>,
}

impl Workspace {
    /// The editor with the keyboard, and the language of its file.
    pub(super) fn focused_editor(&self, window: &Window, cx: &App) -> Option<(Entity<EditorState>, &'static str)> {
        let focused = |editor: &Entity<EditorState>| editor.read(cx).focus_handle(cx).is_focused(window);
        if let Some(split) = &self.split
            && focused(&split.editor)
        {
            let language = self.docs.iter().find(|doc| doc.path == split.path)?.language;
            return Some((split.editor.clone(), language));
        }
        self.docs
            .iter()
            .find(|doc| focused(&doc.editor))
            .map(|doc| (doc.editor.clone(), doc.language))
    }

    pub(super) fn on_extend_selection(
        &mut self,
        _: &ExtendSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((editor, language)) = self.focused_editor(window, cx) else {
            return;
        };
        let (text, selected) = {
            let state = editor.read(cx);
            (state.text().to_string(), state.selected_range())
        };
        let id = editor.entity_id();
        let steps = &mut self.expansions;
        if steps.editor != Some(id) || steps.current.as_ref() != Some(&selected) {
            *steps = Expansions {
                editor: Some(id),
                ..Default::default()
            };
        }
        let Some(next) = expand::extend(language, &text, selected.clone()) else {
            return;
        };
        steps.before.push(selected);
        steps.current = Some(next.clone());
        editor.update(cx, |editor, cx| editor.set_selected_range(next, cx));
    }

    pub(super) fn on_shrink_selection(
        &mut self,
        _: &ShrinkSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((editor, _)) = self.focused_editor(window, cx) else {
            return;
        };
        let selected = editor.read(cx).selected_range();
        let steps = &mut self.expansions;
        if steps.editor != Some(editor.entity_id()) || steps.current.as_ref() != Some(&selected) {
            return;
        }
        let Some(previous) = steps.before.pop() else {
            return;
        };
        steps.current = Some(previous.clone());
        editor.update(cx, |editor, cx| editor.set_selected_range(previous, cx));
    }
}
