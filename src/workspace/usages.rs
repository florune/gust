//! Usages and implementations: Ctrl+click on a declaration, or Alt+F7 /
//! Ctrl+Alt+B anywhere, lists what refers to it in the palette.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;

use super::Workspace;
use super::palette::Mode;
use crate::finder::{self, Hit};
use crate::goindex;

actions!(gust, [FindUsages]);

pub(super) fn init(cx: &mut App) {
    let editor = Some("Input && mode == editor");
    cx.bind_keys([
        KeyBinding::new("alt-f7", FindUsages, editor),
        KeyBinding::new("ctrl-alt-b", FindUsages, editor),
    ]);
}

/// The byte offset of `row` and byte column `col` in `text`.
fn offset_of(text: &str, row: u32, col: u32) -> usize {
    let line_start: usize = text
        .split_inclusive('\n')
        .take(row as usize)
        .map(str::len)
        .sum();
    (line_start + col as usize).min(text.len())
}

impl Workspace {
    pub(super) fn on_find_usages(&mut self, _: &FindUsages, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active.and_then(|ix| self.docs.get(ix)) else {
            return;
        };
        if doc.language != "go" {
            return;
        }
        let (path, offset) = (doc.path.clone(), doc.editor.read(cx).cursor());
        self.find_usages(path, offset, window, cx);
    }

    /// Usages of the name at `row` and byte column `col` of the open file `path`.
    pub(super) fn show_usages_at(
        &mut self,
        path: PathBuf,
        row: u32,
        col: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.iter().find(|doc| doc.path == path) else {
            return;
        };
        let offset = offset_of(&doc.editor.read(cx).text().to_string(), row, col);
        self.find_usages(path, offset, window, cx);
    }

    fn find_usages(&mut self, path: PathBuf, offset: usize, _: &mut Window, cx: &mut Context<Self>) {
        let open: HashMap<PathBuf, String> = self
            .docs
            .iter()
            .filter(|doc| doc.language == "go")
            .map(|doc| (doc.path.clone(), doc.editor.read(cx).text().to_string()))
            .collect();
        let Some(source) = open.get(&path).cloned() else {
            return;
        };
        let root = self.root.clone();
        let known = self.project_files.clone();
        let hidden = self.settings.hidden.clone();
        let index = self.index.clone();
        self.usages_search = cx.spawn(async move |this, cx| {
            let searching = cx.background_executor().spawn(async move {
                let files =
                    known.unwrap_or_else(|| Arc::new(finder::project_files(&root, &hidden)));
                let absolute: Vec<PathBuf> = files.iter().map(|file| root.join(file)).collect();
                let found = goindex::usages(&index, &absolute, &open, &path, &source, offset);
                (files, found)
            });
            let (files, found) = searching.await;
            this.update_in(cx, |this, window, cx| {
                this.project_files = Some(files);
                this.show_usages(found, window, cx);
            })
            .ok();
        });
    }

    fn show_usages(
        &mut self,
        found: Option<goindex::Usages>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(found) = found else {
            return;
        };
        let relative = |file: &Path| {
            file.strip_prefix(&self.root)
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let row = |hit: goindex::Hit, tag: &'static str| Hit {
            path: relative(&hit.file),
            row: hit.row,
            col: hit.col,
            preview: hit.line,
            tag,
        };
        let implementations = found.implementations.len();
        let usages = found.usages.len();
        let hits: Vec<Hit> = found
            .implementations
            .into_iter()
            .map(|hit| row(hit, "实现"))
            .chain(found.usages.into_iter().map(|hit| row(hit, "")))
            .collect();
        match hits.as_slice() {
            [] => {
                self.notice = Some(format!("没有找到 {} 的用法", found.name).into());
                cx.notify();
            }
            // Like GoLand, a single place is simply gone to.
            [only] => {
                let (path, row, col) = (self.root.join(&only.path), only.row, only.col);
                self.open_at(path, row, col, window, cx);
            }
            _ => {
                let mut title = format!("{} · {usages} 处用法", found.name);
                if implementations > 0 {
                    title = format!("{} · {implementations} 个实现 · {usages} 处用法", found.name);
                }
                self.open_palette(Mode::Usages, window, cx);
                if let Some(palette) = &mut self.palette {
                    palette.title = Some(title.into());
                    palette.found = hits.clone();
                    palette.hits = hits;
                }
                cx.notify();
            }
        }
    }
}
