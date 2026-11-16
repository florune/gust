//! Alt+Enter: what can be done at the caret (add json tags, implement an
//! interface, go mod tidy), and live templates expanded with Tab.

use std::path::{Path, PathBuf};

use gpui_kit::*;

use super::Workspace;
use super::palette::Mode;
use crate::finder::Hit;
use crate::generate;
use crate::goindex;

actions!(gust, [ShowIntentions, TabOrTemplate]);

pub(super) fn init(cx: &mut App) {
    let editor = Some("Input && mode == editor");
    cx.bind_keys([
        KeyBinding::new("alt-enter", ShowIntentions, editor),
        // Registered after the editor's own Tab, so this one is used; it
        // falls back to the editor's indent when no template applies.
        KeyBinding::new("tab", TabOrTemplate, editor),
    ]);
}

const JSON_TAGS: &str = "json-tags";
const IMPLEMENT: &str = "implement";
const TIDY: &str = "tidy";

fn intention(id: &str, label: &str) -> Hit {
    Hit {
        path: id.to_string(),
        row: 0,
        col: 0,
        preview: label.to_string(),
        tag: "",
        detail: String::new(),
    }
}

impl Workspace {
    /// The open file whose editor has the keyboard: its index, text and caret.
    fn caret_doc(&self, window: &Window, cx: &App) -> Option<(usize, String, usize)> {
        let (editor, _) = self.focused_editor(window, cx)?;
        let ix = self
            .docs
            .iter()
            .position(|doc| doc.editor == editor)
            .or_else(|| {
                let split = self.split.as_ref()?;
                self.docs.iter().position(|doc| doc.path == split.path)
            })?;
        let state = editor.read(cx);
        Some((ix, state.text().to_string(), state.cursor()))
    }

    pub(super) fn on_show_intentions(&mut self, _: &ShowIntentions, window: &mut Window, cx: &mut Context<Self>) {
        let Some((ix, source, offset)) = self.caret_doc(window, cx) else {
            return;
        };
        let doc = &self.docs[ix];
        let go_mod = doc.path.file_name().is_some_and(|name| name == "go.mod");
        if doc.language != "go" && !go_mod {
            return;
        }
        let mut hits = Vec::new();
        if doc.language == "go" {
            if generate::can_add_json_tags(&source, offset) {
                hits.push(intention(JSON_TAGS, "添加 json 标签"));
            }
            if let Some(name) = goindex::type_at(&source, offset) {
                hits.push(intention(IMPLEMENT, &format!("为 {name} 实现接口…")));
            }
        }
        hits.push(intention(TIDY, "运行 go mod tidy"));
        self.intention_target = Some((doc.path.clone(), offset));
        self.open_palette(Mode::Intentions, window, cx);
        if let Some(palette) = &mut self.palette {
            palette.found = hits.clone();
            palette.hits = hits;
        }
        cx.notify();
    }

    /// Carry out the intention `id` picked from the list.
    pub(super) fn run_intention(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some((path, offset)) = self.intention_target.clone() else {
            return;
        };
        let Some(ix) = self.docs.iter().position(|doc| doc.path == path) else {
            return;
        };
        let editor = self.docs[ix].editor.clone();
        let source = editor.read(cx).text().to_string();
        match id {
            JSON_TAGS => {
                if let Some(tagged) = generate::json_tags(&source, offset) {
                    self.apply_as_one_edit(&editor, &source, &tagged, None, window, cx);
                    // gofmt lines the tags up.
                    self.format_doc(ix, false, window, cx);
                }
            }
            IMPLEMENT => self.choose_interface(window, cx),
            TIDY => {
                let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| self.root.clone());
                self.run_go_tool(&dir, &["mod", "tidy"], "go mod tidy", window, cx);
            }
            _ => {}
        }
    }

    /// The interfaces of the project, to pick the one to implement.
    fn choose_interface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let files = self
            .project_files
            .clone()
            .unwrap_or_else(|| std::sync::Arc::new(crate::finder::project_files(&self.root, &self.settings.hidden)));
        let mut dirs: Vec<PathBuf> = files
            .iter()
            .filter(|file| file.ends_with(".go"))
            .filter_map(|file| self.root.join(file).parent().map(Path::to_path_buf))
            .collect();
        dirs.sort();
        dirs.dedup();
        let hits: Vec<Hit> = goindex::interfaces(&self.index, &dirs)
            .into_iter()
            .map(|interface| Hit {
                path: interface.file.to_string_lossy().into_owned(),
                row: interface.row,
                col: interface.col,
                preview: interface.name,
                tag: "",
                detail: super::relative_path(&self.root, &interface.file),
            })
            .collect();
        if hits.is_empty() {
            self.notice = Some("项目中没有接口".into());
            cx.notify();
            return;
        }
        self.open_palette(Mode::Interfaces, window, cx);
        if let Some(palette) = &mut self.palette {
            palette.title = Some("选择要实现的接口".into());
            palette.found = hits.clone();
            palette.hits = hits;
        }
        cx.notify();
    }

    /// Add stubs for what the type at the caret lacks of `interface`.
    pub(super) fn implement_interface(
        &mut self,
        interface_file: PathBuf,
        interface: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((path, offset)) = self.intention_target.clone() else {
            return;
        };
        let Some(ix) = self.docs.iter().position(|doc| doc.path == path) else {
            return;
        };
        let editor = self.docs[ix].editor.clone();
        let source = editor.read(cx).text().to_string();
        let Some(type_name) = goindex::type_at(&source, offset) else {
            return;
        };
        match goindex::implement(&self.index, &path, &source, &type_name, &interface_file, &interface) {
            Some((at, stubs)) => {
                let after = format!("{}{stubs}{}", &source[..at], &source[at..]);
                self.apply_as_one_edit(&editor, &source, &after, Some(at + 2..at + 2), window, cx);
                self.activate(ix, window, cx);
            }
            None => {
                self.notice = Some(format!("{type_name} 已实现 {interface} 的全部方法").into());
                cx.notify();
            }
        }
    }

    /// Tab after a template's name expands it; otherwise Tab indents.
    pub(super) fn on_tab_or_template(&mut self, _: &TabOrTemplate, window: &mut Window, cx: &mut Context<Self>) {
        if !self.expand_template(window, cx) {
            window.defer(cx, |window, cx| {
                window.dispatch_action(Box::new(gpui_kit::component::input::IndentInline), cx)
            });
        }
    }

    /// Expand the template whose name is before the caret; false if none is.
    fn expand_template(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((editor, language)) = self.focused_editor(window, cx) else {
            return false;
        };
        if language != "go" {
            return false;
        }
        let (source, selection, menu_open) = {
            let state = editor.read(cx);
            (state.text().to_string(), state.selected_range(), state.completion_menu_state().open)
        };
        if !selection.is_empty() || menu_open {
            return false;
        }
        let caret = selection.start;
        let line_start = source[..caret].rfind('\n').map_or(0, |ix| ix + 1);
        let before = &source[line_start..caret];
        let word_start = before
            .rfind(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
            .map_or(0, |ix| ix + 1);
        let word = &before[word_start..];
        let indent: String = before.chars().take_while(|ch| ch.is_whitespace()).collect();
        let Some(expansion) = generate::expand_template(word, &source, caret, &indent) else {
            return false;
        };
        let marker = expansion.find('$').unwrap_or(expansion.len());
        let inserted = expansion.replacen('$', "", 1);
        let start = line_start + word_start;
        let after = format!("{}{inserted}{}", &source[..start], &source[caret..]);
        let at = start + marker;
        self.apply_as_one_edit(&editor, &source, &after, Some(at..at), window, cx);
        true
    }
}
