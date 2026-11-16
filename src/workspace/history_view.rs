//! Local history of a file: its kept versions in a list, a compare view of a
//! version against the current text, and restoring a version.

use std::path::{Path, PathBuf};

use gpui_kit::*;

use super::Workspace;
use super::compare_view::{Entry, Source};
use super::palette::Mode;
use crate::finder::Hit;
use crate::local_history;

impl Workspace {
    /// Keep `text`, the content `file` has before a change, in local history.
    pub(super) fn snapshot_before_change(&self, file: &Path, text: &str, label: &str) {
        local_history::record(&self.root, file, text, label);
    }

    /// The current text of `file`: in its editor when open, otherwise on disk.
    fn current_text(&self, file: &Path, cx: &App) -> Option<String> {
        match self.docs.iter().find(|doc| doc.path == file) {
            Some(doc) => Some(doc.editor.read(cx).text().to_string()),
            None => std::fs::read_to_string(file)
                .ok()
                .map(|text| text.replace("\r\n", "\n")),
        }
    }

    /// List the kept versions of `file`, newest first.
    pub(super) fn show_local_history(&mut self, file: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let versions = local_history::list(&self.root, &file);
        let hits: Vec<Hit> = versions
            .iter()
            .map(|version| Hit {
                path: version.id.to_string_lossy().into_owned(),
                row: 0,
                col: 0,
                preview: format!(
                    "{} · {}",
                    super::log::when(version.time),
                    chrono::DateTime::from_timestamp(version.time, 0)
                        .map(|time| time.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_default()
                ),
                tag: "",
                detail: format!("{} · {} 行", version.label, version.lines),
            })
            .collect();
        let name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.open_palette(Mode::History, window, cx);
        if let Some(palette) = &mut self.palette {
            palette.title = Some(format!("{name} · 本地历史 · {} 个版本", hits.len()).into());
            palette.found = hits.clone();
            palette.hits = hits;
            palette.history_file = Some(file);
        }
        cx.notify();
    }

    /// Compare a kept version of `file` with its current text.
    pub(super) fn compare_with_version(
        &mut self,
        file: PathBuf,
        version: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(current) = self.current_text(&file, cx) else {
            return;
        };
        let entry = Entry {
            path: super::relative_path(&self.root, &file),
            old_path: None,
            status: 'M',
        };
        let source = Source::Version {
            file,
            version,
            current,
        };
        self.show_compare(source, vec![entry], 0, window, cx);
    }

    /// Make the kept `version` the text of `file`; what it replaces is kept too.
    pub(super) fn restore_version(
        &mut self,
        file: &Path,
        version: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(text) = local_history::load(version) else {
            self.notice = Some("无法读取该版本".into());
            cx.notify();
            return;
        };
        if let Some(current) = self.current_text(file, cx) {
            self.snapshot_before_change(file, &current, "恢复前");
        }
        match self.docs.iter().find(|doc| doc.path == file) {
            Some(doc) => {
                let editor = doc.editor.clone();
                let before = editor.read(cx).text().to_string();
                self.apply_as_one_edit(&editor, &before, &text, None, window, cx);
            }
            None => {
                if let Err(err) = std::fs::write(file, &text) {
                    self.notice = Some(format!("无法写入：{err}").into());
                }
            }
        }
        self.close_diff(window, cx);
        self.open(file.to_path_buf(), window, cx);
    }
}
