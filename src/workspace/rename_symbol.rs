//! Rename a Go symbol (Shift+F6): its declaration and every use the index
//! resolves to it, in open files and on disk.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::*;

use super::Workspace;
use super::file_ops::FileAction;
use crate::finder;
use crate::goindex::{self, Location};

actions!(gust, [RenameSymbol]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "shift-f6",
        RenameSymbol,
        Some("Input && mode == editor"),
    )]);
}

/// Every place a name is written, to rename them together.
#[derive(Debug, PartialEq)]
pub(super) struct RenamePlan {
    pub name: String,
    /// Line and byte column of each occurrence, by file.
    pub places: BTreeMap<PathBuf, Vec<(u32, u32)>>,
}

impl RenamePlan {
    pub fn count(&self) -> usize {
        self.places.values().map(Vec::len).sum()
    }
}

/// Whether `name` can name a Go declaration.
fn is_identifier(name: &str) -> bool {
    const KEYWORDS: &[&str] = &[
        "break", "case", "chan", "const", "continue", "default", "defer", "else", "fallthrough",
        "for", "func", "go", "goto", "if", "import", "interface", "map", "package", "range",
        "return", "select", "struct", "switch", "type", "var",
    ];
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_alphanumeric() || ch == '_')
        && !KEYWORDS.contains(&name)
}

/// `text` with the name at each of `places` replaced by `to`.
fn renamed(text: &str, places: &[(u32, u32)], from: &str, to: &str) -> String {
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(ix, _)| ix + 1))
        .collect();
    let mut offsets: Vec<usize> = places
        .iter()
        .filter_map(|(row, col)| Some(starts.get(*row as usize)? + *col as usize))
        .filter(|offset| text.get(*offset..*offset + from.len()) == Some(from))
        .collect();
    offsets.sort_unstable();
    offsets.dedup();
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for offset in offsets {
        out.push_str(&text[at..offset]);
        out.push_str(to);
        at = offset + from.len();
    }
    out.push_str(&text[at..]);
    out
}

impl Workspace {
    pub(super) fn on_rename_symbol(&mut self, _: &RenameSymbol, window: &mut Window, cx: &mut Context<Self>) {
        let Some((editor, language)) = self.focused_editor(window, cx) else {
            return;
        };
        if language != "go" {
            return;
        }
        let Some(path) = self
            .docs
            .iter()
            .find(|doc| doc.editor == editor)
            .map(|doc| doc.path.clone())
            .or_else(|| self.split.as_ref().map(|split| split.path.clone()))
        else {
            return;
        };
        let (source, offset) = {
            let state = editor.read(cx);
            (state.text().to_string(), state.cursor())
        };
        let open: HashMap<PathBuf, String> = self
            .docs
            .iter()
            .filter(|doc| doc.language == "go")
            .map(|doc| (doc.path.clone(), doc.editor.read(cx).text().to_string()))
            .collect();
        let root = self.root.clone();
        let known = self.project_files.clone();
        let hidden = self.settings.hidden.clone();
        let index = self.index.clone();
        self.usages_search = cx.spawn_in(window, async move |this, cx| {
            let searching = cx.background_executor().spawn(async move {
                let files = known.unwrap_or_else(|| Arc::new(finder::project_files(&root, &hidden)));
                let absolute: Vec<PathBuf> = files.iter().map(|file| root.join(file)).collect();
                goindex::usages(&index, &absolute, &open, &path, &source, offset)
            });
            let Some(found) = searching.await else {
                return;
            };
            let mut places: BTreeMap<PathBuf, Vec<(u32, u32)>> = BTreeMap::new();
            if let Some(Location { file, row, col }) = &found.declaration {
                places.entry(file.clone()).or_default().push((*row, *col));
            }
            for hit in &found.usages {
                places.entry(hit.file.clone()).or_default().push((hit.row, hit.col));
            }
            let plan = RenamePlan {
                name: found.name,
                places,
            };
            this.update_in(cx, |this, window, cx| {
                this.open_file_prompt(FileAction::RenameSymbol(Arc::new(plan)), window, cx)
            })
            .ok();
        });
    }

    /// Rename every place of `plan` to `to`: open files in their editors, so
    /// the change can be undone there; other files on disk.
    pub(super) fn apply_rename(
        &mut self,
        plan: &RenamePlan,
        to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let to = to.trim();
        if !is_identifier(to) {
            return Err("名称无效".into());
        }
        if to == plan.name {
            return Ok(());
        }
        for (file, places) in &plan.places {
            if let Some(doc) = self.docs.iter().find(|doc| &doc.path == file) {
                let editor = doc.editor.clone();
                let before = editor.read(cx).text().to_string();
                let after = renamed(&before, places, &plan.name, to);
                self.apply_as_one_edit(&editor, &before, &after, None, window, cx);
                continue;
            }
            let raw = std::fs::read_to_string(file).map_err(|err| format!("无法读取：{err}"))?;
            let crlf = raw.contains("\r\n");
            let text = if crlf { raw.replace("\r\n", "\n") } else { raw };
            self.snapshot_before_change(file, &text, "重命名前");
            let after = renamed(&text, places, &plan.name, to);
            let after = if crlf { after.replace('\n', "\r\n") } else { after };
            std::fs::write(file, after).map_err(|err| format!("无法写入：{err}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{is_identifier, renamed};

    #[test]
    fn only_the_places_given_are_renamed() {
        let text = "func load() {}\n\nfunc main() {\n\tload()\n\tdownload()\n}\n";
        let places = [(0, 5), (3, 1)];
        assert_eq!(
            renamed(text, &places, "load", "fetch"),
            "func fetch() {}\n\nfunc main() {\n\tfetch()\n\tdownload()\n}\n"
        );
        // A place that no longer holds the name is left alone.
        assert_eq!(renamed("x := 1\n", &[(0, 0)], "load", "fetch"), "x := 1\n");
    }

    #[test]
    fn new_names_must_be_go_identifiers() {
        assert!(is_identifier("fetchItem"));
        assert!(is_identifier("_x2"));
        assert!(!is_identifier("2x"));
        assert!(!is_identifier("a-b"));
        assert!(!is_identifier("func"));
        assert!(!is_identifier(""));
    }
}
