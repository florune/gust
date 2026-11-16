//! The files that were open, where their carets were, and the files opened
//! last, kept with the project so that reopening it carries on from there.

use std::path::PathBuf;

use gpui_kit::component::RopeExt as _;
use gpui_kit::*;

use super::Workspace;
use crate::settings::{self, OpenFile, Session};

impl Workspace {
    /// The session as it is now.
    fn session(&self, cx: &App) -> Session {
        let relative = |path: &std::path::Path| super::relative_path(&self.root, path);
        let open = self
            .docs
            .iter()
            .map(|doc| {
                let editor = doc.editor.read(cx);
                let position = editor.cursor_position();
                let line = editor.text().slice_line(position.line as usize).to_string();
                // The position counts UTF-16 units; the session keeps bytes.
                let mut units = 0;
                let mut col = line.len();
                for (ix, ch) in line.char_indices() {
                    if units >= position.character as usize {
                        col = ix;
                        break;
                    }
                    units += ch.len_utf16();
                }
                OpenFile {
                    path: relative(&doc.path),
                    row: position.line,
                    col: col as u32,
                }
            })
            .collect();
        Session {
            open,
            active: self
                .active
                .and_then(|ix| self.docs.get(ix))
                .map(|doc| relative(&doc.path)),
            recent: self.recent_files.iter().take(30).map(|path| relative(path)).collect(),
        }
    }

    /// Save the session, when it changed.
    pub(super) fn remember_session(&mut self, cx: &App) {
        let session = self.session(cx);
        if session == self.settings.session {
            return;
        }
        self.settings.session = session;
        // Not a reason to bother the user: the session is a convenience.
        let _ = settings::save(&self.root, &self.settings);
    }

    /// Open what was open when the project was last left.
    pub(super) fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.settings.session.clone();
        let root = self.root.clone();
        let absolute = |path: &str| -> PathBuf { root.join(path) };
        self.navigating = true;
        for file in &session.open {
            let path = absolute(&file.path);
            if path.is_file() {
                self.open_at(path, file.row, file.col, window, cx);
            }
        }
        if let Some(active) = session.active.as_deref().map(absolute)
            && let Some(ix) = self.docs.iter().position(|doc| doc.path == active)
        {
            self.activate(ix, window, cx);
        }
        self.navigating = false;
        self.recent_files = session
            .recent
            .iter()
            .map(|path| absolute(path))
            .filter(|path| path.is_file())
            .collect();
        // The files reopened are not jumps to go back from.
        self.back.clear();
        self.forward.clear();
    }
}
