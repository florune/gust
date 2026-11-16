//! GoLand's editing keys: Ctrl+/ comments, Ctrl+D duplicates, Ctrl+Y deletes
//! the line, Ctrl+Shift+↑/↓ moves lines, Ctrl+Shift+Z redoes, Ctrl+R
//! replaces, Ctrl+E lists the files opened last.

use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Redo, Replace};
use gpui_kit::*;

use super::Workspace;
use super::palette::Mode;
use crate::finder::Hit;
use crate::lines::{self, Edit};

actions!(
    gust,
    [ToggleComment, DuplicateLine, DeleteLine, MoveLineUp, MoveLineDown, RecentFiles]
);

/// How many files Ctrl+E remembers.
const RECENT_FILES: usize = 50;

pub(super) fn init(cx: &mut App) {
    // In the editor's own context, registered after its keys, so these win.
    let editor = Some("Input && mode == editor");
    cx.bind_keys([
        KeyBinding::new("ctrl-/", ToggleComment, editor),
        KeyBinding::new("ctrl-d", DuplicateLine, editor),
        KeyBinding::new("ctrl-y", DeleteLine, editor),
        KeyBinding::new("ctrl-shift-z", Redo, editor),
        KeyBinding::new("ctrl-shift-up", MoveLineUp, editor),
        KeyBinding::new("ctrl-shift-down", MoveLineDown, editor),
        KeyBinding::new("alt-shift-up", MoveLineUp, editor),
        KeyBinding::new("alt-shift-down", MoveLineDown, editor),
        KeyBinding::new("ctrl-r", Replace, editor),
        KeyBinding::new("ctrl-e", RecentFiles, editor),
        KeyBinding::new("ctrl-e", RecentFiles, Some("Workspace")),
    ]);
}

impl Workspace {
    /// Change the focused editor's text and selection by `edit`.
    fn edit_lines(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&str, Range<usize>, &'static str) -> Option<Edit>,
    ) {
        let Some((editor, language)) = self.focused_editor(window, cx) else {
            return;
        };
        let (text, selected) = {
            let state = editor.read(cx);
            (state.text().to_string(), state.selected_range())
        };
        let Some(edit) = edit(&text, selected, language) else {
            return;
        };
        self.apply_as_one_edit(&editor, &text, &edit.text, Some(edit.selection), window, cx);
    }

    /// Turn `editor`'s text from `before` into `after` in one edit, so one
    /// Ctrl+Z takes it all back; then select `selection` if given.
    pub(super) fn apply_as_one_edit(
        &self,
        editor: &Entity<gpui_kit::component::input::EditorState>,
        before: &str,
        after: &str,
        selection: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(change) = crate::format::single_edit(before, after) else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.apply_lsp_edits(&vec![change], window, cx);
            if *editor.text() != after {
                editor.set_value(after.to_string(), window, cx);
            }
            if let Some(selection) = selection {
                editor.set_selected_range(selection, cx);
            }
        });
    }

    pub(super) fn on_toggle_comment(&mut self, _: &ToggleComment, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_lines(window, cx, |text, selected, language| {
            let prefix = lines::comment_prefix(language)?;
            Some(lines::toggle_comment(text, selected, prefix))
        });
    }

    pub(super) fn on_duplicate_line(&mut self, _: &DuplicateLine, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_lines(window, cx, |text, selected, _| Some(lines::duplicate(text, selected)));
    }

    pub(super) fn on_delete_line(&mut self, _: &DeleteLine, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_lines(window, cx, |text, selected, _| Some(lines::delete_lines(text, selected)));
    }

    pub(super) fn on_move_line_up(&mut self, _: &MoveLineUp, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_lines(window, cx, |text, selected, _| lines::move_lines(text, selected, true));
    }

    pub(super) fn on_move_line_down(&mut self, _: &MoveLineDown, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_lines(window, cx, |text, selected, _| lines::move_lines(text, selected, false));
    }

    /// `path` was just shown: it goes to the top of the recent files.
    pub(super) fn note_recent_file(&mut self, path: &Path) {
        self.recent_files.retain(|known| known != path);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(RECENT_FILES);
    }

    pub(super) fn on_recent_files(&mut self, _: &RecentFiles, window: &mut Window, cx: &mut Context<Self>) {
        let files: Vec<PathBuf> = self
            .recent_files
            .iter()
            .filter(|path| path.is_file())
            .cloned()
            .collect();
        let hits: Vec<Hit> = files
            .iter()
            .map(|path| Hit {
                path: super::relative_path(&self.root, path),
                row: 0,
                col: 0,
                preview: String::new(),
                tag: "",
                detail: String::new(),
            })
            .collect();
        // The file before this one is picked, so Ctrl+E, Enter goes back to it.
        let current = self.active.and_then(|ix| self.docs.get(ix)).map(|doc| doc.path.clone());
        let selected = usize::from(hits.len() > 1 && files.first() == current.as_ref());
        self.open_palette(Mode::Recent, window, cx);
        if let Some(palette) = &mut self.palette {
            palette.title = Some("最近的文件".into());
            palette.found = hits.clone();
            palette.hits = hits;
            palette.selected = selected;
        }
        cx.notify();
    }
}
