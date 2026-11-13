use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{
    EditorState, GoToDefinition, GutterDecorationCollection, InputEvent, Position,
    ShowCompletions, TabSize, TextareaState, highlight_characters,
};
use gpui_kit::component::{ActiveTheme as _, Theme, ThemeMode};
use gpui_kit::*;

use crate::complete::{GoFile, uri_to_path, usages_target};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::format;
use crate::git;
use crate::goindex::Index;
use crate::lang;
use crate::settings::{self, ProjectSettings};
use crate::terminal::{TerminalEvent, TerminalView};

mod autoscroll;
mod changes;
mod compare_view;
mod layout;
mod log;
mod markdown;
mod palette;
mod run_panel;
mod tabs;
mod usages;

use changes::SidebarTab;
use compare_view::DiffTab;
use layout::BottomTab;
use markdown::{DocView, Preview};

actions!(
    gust,
    [
        Save,
        Format,
        CloseTab,
        ToggleSidebar,
        ShowCommit,
        ToggleTheme,
        OpenFolder,
        ToggleTerminal,
        NewTerminal,
        ShowGitLog
    ]
);

/// Keys follow GoLand where it has a binding, since that is what this replaces.
///
/// They are bound in the workspace's context rather than globally: a deeper
/// context, such as the terminal, can then claim a key for itself.
pub fn init(cx: &mut App) {
    let workspace = Some("Workspace");
    let editor = Some("Input && mode == editor");
    cx.bind_keys([
        KeyBinding::new("ctrl-s", Save, workspace),
        KeyBinding::new("ctrl-alt-l", Format, workspace),
        KeyBinding::new("shift-alt-f", Format, workspace),
        KeyBinding::new("ctrl-w", CloseTab, workspace),
        KeyBinding::new("ctrl-f4", CloseTab, workspace),
        KeyBinding::new("alt-1", ToggleSidebar, workspace),
        KeyBinding::new("alt-0", ShowCommit, workspace),
        KeyBinding::new("ctrl-o", OpenFolder, workspace),
        KeyBinding::new("ctrl-`", ToggleTerminal, workspace),
        KeyBinding::new("alt-f12", ToggleTerminal, workspace),
        KeyBinding::new("ctrl-shift-`", NewTerminal, workspace),
        KeyBinding::new("alt-9", ShowGitLog, workspace),
        KeyBinding::new("ctrl-b", GoToDefinition, editor),
        KeyBinding::new("f12", GoToDefinition, editor),
        KeyBinding::new("ctrl-space", ShowCompletions, editor),
    ]);
    palette::init(cx);
    markdown::init(cx);
    compare_view::init(cx);
    usages::init(cx);
    run_panel::init(cx);
}

/// One open file.
struct Doc {
    path: PathBuf,
    title: SharedString,
    language: &'static str,
    editor: Entity<EditorState>,
    /// The text as it is on disk, with `\n` line endings.
    saved: String,
    /// The file used `\r\n` when it was read; written back the same way.
    crlf: bool,
    dirty: bool,
    /// The committed text, which the change marks compare against.
    head: Option<String>,
    gutter: Option<GutterDecorationCollection>,
    /// Source, preview, or both: Markdown only.
    view: DocView,
    preview: Option<Preview>,
    _subscriptions: Vec<Subscription>,
}

struct TerminalTab {
    name: SharedString,
    view: Entity<TerminalView>,
    _subscription: Subscription,
}

pub struct Workspace {
    root: PathBuf,
    focus: FocusHandle,
    tree: Entity<FileTree>,
    docs: Vec<Doc>,
    active: Option<usize>,
    sidebar_visible: bool,
    sidebar_tab: SidebarTab,
    git: Option<git::Status>,
    branches: Vec<String>,
    git_refresh: Task<()>,
    /// A commit, push, pull, or branch switch is running.
    git_busy: bool,
    /// Changed files left out of the next commit.
    excluded: HashSet<String>,
    commit_message: Entity<TextareaState>,
    diff: Option<DiffTab>,
    /// The diff tab, not a file, is what the editor area shows.
    diff_active: bool,
    /// Where the diff tab sits among the file tabs.
    diff_slot: usize,
    /// The window, for work finished in the background that builds views.
    window: AnyWindowHandle,
    watching: Task<()>,
    _watcher: Option<notify::RecommendedWatcher>,
    palette: Option<palette::Palette>,
    /// The usages search in flight.
    usages_search: Task<()>,
    /// Run configurations, the build, and the program's console.
    run: run_panel::RunState,
    /// The project's files, listed once and kept until something changes on disk.
    project_files: Option<Arc<Vec<String>>>,
    terminals: Vec<TerminalTab>,
    active_terminal: usize,
    /// The bottom panel is shown, with `bottom_tab` in it.
    terminal_visible: bool,
    bottom_tab: BottomTab,
    log: Option<log::GitLog>,
    /// How many terminals were ever opened, which numbers the next one.
    terminals_opened: usize,
    memory_mb: u64,
    /// Why the last save or format did not go through, shown in the status bar.
    notice: Option<SharedString>,
    index: Arc<Index>,
    settings: ProjectSettings,
    /// Projects opened before, for the project menu.
    recent: Vec<crate::recent::RecentProject>,
    /// How a newly opened Markdown file is shown: the way the last one was.
    markdown_view: DocView,
    autoscroll: Option<autoscroll::Autoscroll>,
    /// The hidden paths, as the file watcher's thread sees them.
    watched_hidden: Arc<std::sync::RwLock<Vec<String>>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(root: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = settings::load(&root);
        let hidden = settings.hidden.clone();
        let tree = cx.new(|_| FileTree::new(root.clone(), hidden));
        let subscriptions = vec![
            cx.subscribe_in(
                &tree,
                window,
                |this, _, event: &FileTreeEvent, window, cx| match event {
                    FileTreeEvent::Open(path) => this.open(path.clone(), window, cx),
                    FileTreeEvent::Hide(path) => this.set_path_hidden(path, true, cx),
                    FileTreeEvent::Unhide(path) => this.set_path_hidden(path, false, cx),
                },
            ),
            // Like GoLand: leaving the window saves, so tools outside it see
            // what the editor shows.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.save_all(cx);
                    this.stop_autoscroll(cx);
                }
            }),
        ];
        let commit_message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("提交说明")
                .auto_grow(2, 6)
        });

        cx.spawn(async move |this, cx| {
            loop {
                let updated = this.update(cx, |this, cx| {
                    let mb = memory_stats::memory_stats()
                        .map(|m| (m.physical_mem / (1024 * 1024)) as u64)
                        .unwrap_or(0);
                    if mb != this.memory_mb {
                        this.memory_mb = mb;
                        cx.notify();
                    }
                });
                if updated.is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(2)).await;
            }
        })
        .detach();

        let recent = crate::recent::record(&root);
        let mut this = Self {
            recent,
            root,
            focus: cx.focus_handle(),
            tree,
            docs: Vec::new(),
            active: None,
            sidebar_visible: true,
            sidebar_tab: SidebarTab::Files,
            git: None,
            branches: Vec::new(),
            git_refresh: Task::ready(()),
            git_busy: false,
            excluded: HashSet::new(),
            commit_message,
            diff: None,
            diff_active: false,
            diff_slot: 0,
            window: window.window_handle(),
            watching: Task::ready(()),
            _watcher: None,
            palette: None,
            usages_search: Task::ready(()),
            run: run_panel::RunState::new(),
            project_files: None,
            terminals: Vec::new(),
            active_terminal: 0,
            terminal_visible: false,
            bottom_tab: BottomTab::Terminal,
            log: None,
            terminals_opened: 0,
            memory_mb: 0,
            notice: None,
            index: Arc::new(Index::new()),
            markdown_view: DocView::Preview,

            autoscroll: None,
            watched_hidden: Arc::new(std::sync::RwLock::new(settings.hidden.clone())),
            settings,
            _subscriptions: subscriptions,
        };
        this.refresh_git(cx);
        this.watch(window, cx);
        // Keys reach the workspace through whatever inside it has focus.
        this.focus.focus(window, cx);
        this
    }

    /// Follow changes made to the folder by anything else: an AI agent in the
    /// terminal, git, a build.
    fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        use notify::Watcher as _;

        // Tests call `files_changed` themselves: a watcher thread waking the
        // app at arbitrary times would make them depend on timing.
        if cfg!(test) {
            return;
        }

        let (sender, receiver) = async_channel::bounded::<()>(1);
        let hidden = self.watched_hidden.clone();
        let root = self.root.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else {
                return;
            };
            let hidden = hidden.read().map(|hidden| hidden.clone()).unwrap_or_default();
            // Reads change nothing, git's object store and lock files change
            // constantly without changing what is shown, and hidden paths
            // are not shown at all.
            let noise = event.kind.is_access()
                || event.paths.iter().all(|path| {
                    path.extension().is_some_and(|ext| ext == "lock")
                        || path
                            .components()
                            .zip(path.components().skip(1))
                            .any(|(a, b)| a.as_os_str() == ".git" && b.as_os_str() == "objects")
                        || path
                            .components()
                            .any(|part| part.as_os_str() == settings::LOCAL_DIR)
                        || path.strip_prefix(&root).is_ok_and(|relative| {
                            settings::is_hidden(
                                &hidden,
                                &relative.to_string_lossy().replace('\\', "/"),
                            )
                        })
                });
            if !noise {
                let _ = sender.try_send(());
            }
        });
        self._watcher = watcher.ok().and_then(|mut watcher| {
            watcher
                .watch(&self.root, notify::RecursiveMode::Recursive)
                .ok()?;
            Some(watcher)
        });
        self.watching = cx.spawn_in(window, async move |this, cx| {
            while receiver.recv().await.is_ok() {
                // Tools write in bursts; act once, after the burst.
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                while receiver.try_recv().is_ok() {}
                let updated =
                    this.update_in(cx, |this, window, cx| this.files_changed(window, cx));
                if updated.is_err() {
                    break;
                }
            }
        });
    }

    fn files_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_files = None;
        self.tree.update(cx, |tree, cx| {
            tree.reload();
            cx.notify();
        });
        for ix in 0..self.docs.len() {
            self.reload_doc(ix, window, cx);
        }
        self.refresh_git(cx);
    }

    /// Take over what is on disk for an open file that changed there.
    fn reload_doc(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let doc = &self.docs[ix];
        let Some(raw) = std::fs::read(&doc.path)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            return;
        };
        let crlf = raw.contains("\r\n");
        let on_disk = if crlf { raw.replace("\r\n", "\n") } else { raw };
        if on_disk == doc.saved {
            return;
        }
        let editor = doc.editor.clone();
        if doc.dirty {
            // Unsaved edits win; say so, since saving will overwrite the file.
            self.notice = Some(format!("{} 在磁盘上被修改了，保存会覆盖它", doc.title).into());
        } else {
            let before = editor.read(cx).text().to_string();
            replace_text(&editor, &before, &on_disk, window, cx);
        }
        let doc = &mut self.docs[ix];
        doc.saved = on_disk;
        doc.crlf = crlf;
        self.refresh_dirty(&editor, cx);
        cx.notify();
    }

    /// Hide a file or folder from this project's views, or show it again.
    fn set_path_hidden(&mut self, path: &Path, hide: bool, cx: &mut Context<Self>) {
        let relative = relative_path(&self.root, path);
        if relative.is_empty() {
            return;
        }
        self.settings.hidden.retain(|entry| entry != &relative);
        if hide {
            // Entries inside the newly hidden folder are covered by it now.
            let prefix = format!("{relative}/");
            self.settings.hidden.retain(|entry| !entry.starts_with(&prefix));
            self.settings.hidden.push(relative);
            self.settings.hidden.sort();
        }
        if let Err(reason) = settings::save(&self.root, &self.settings) {
            self.notice = Some(reason.into());
        }
        if let Ok(mut watched) = self.watched_hidden.write() {
            *watched = self.settings.hidden.clone();
        }
        self.project_files = None;
        let hidden = self.settings.hidden.clone();
        self.tree.update(cx, |tree, cx| tree.set_hidden(hidden, cx));
        cx.notify();
    }

    fn save_all(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.docs.len() {
            if self.docs[ix].dirty {
                self.write_doc(ix, cx);
            }
        }
    }

    fn project_name(&self) -> SharedString {
        self.root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.root.to_string_lossy().into_owned())
            .into()
    }

    pub fn open(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.docs.iter().position(|d| d.path == path) {
            self.activate(ix, window, cx);
            return;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let Ok(raw) = String::from_utf8(bytes) else {
            // Binary or non-UTF-8 content is not something to edit as text.
            return;
        };
        let crlf = raw.contains("\r\n");
        let text = if crlf { raw.replace("\r\n", "\n") } else { raw };
        let language = lang::language_for(&path);

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: matches!(language, "go" | "make"),
                })
                .soft_wrap(false)
                .default_value(text.clone())
        });
        if language == "go" {
            let go = Rc::new(GoFile {
                index: self.index.clone(),
                path: path.clone(),
            });
            let workspace = cx.entity().downgrade();
            editor.update(cx, |editor, _| {
                let lsp = editor.lsp_mut();
                lsp.completion_provider = Some(go.clone());
                lsp.definition_provider = Some(go);
                lsp.completion_menu.max_width = px(560.);
                lsp.set_completion_highlighter(Rc::new(highlight_characters));
                // Definitions may be in another file, which only the workspace can show.
                // A declaration "goes to" its usages instead.
                lsp.show_document = Some(Rc::new(move |params, window, cx| {
                    let usages = usages_target(&params.uri);
                    let Some(path) = usages.clone().or_else(|| uri_to_path(&params.uri)) else {
                        return true;
                    };
                    let start = params.selection.map(|range| range.start).unwrap_or_default();
                    let workspace = workspace.clone();
                    // Deferred: the editor asking is still being updated.
                    window.defer(cx, move |window, cx| {
                        workspace
                            .update(cx, |workspace, cx| {
                                if usages.is_some() {
                                    workspace.show_usages_at(
                                        path,
                                        start.line,
                                        start.character,
                                        window,
                                        cx,
                                    )
                                } else {
                                    workspace.open_at(path, start.line, start.character, window, cx)
                                }
                            })
                            .ok();
                    });
                    true
                }));
            });
        }
        let subscriptions = vec![
            cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refresh_dirty(&editor, cx);
                }
            }),
            // Scrolling the source moves a split preview along with it.
            cx.observe(&editor, |this, editor, cx| {
                if let Some(ix) = this.docs.iter().position(|doc| doc.editor == editor) {
                    this.follow_source(ix, cx);
                }
                cx.notify()
            }),
        ];
        let preview = (language == "markdown").then(|| Preview::new(&text, &editor, cx));

        self.docs.push(Doc {
            title: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
                .into(),
            path,
            language,
            editor,
            saved: text,
            crlf,
            dirty: false,
            head: None,
            gutter: None,
            view: if language == "markdown" {
                self.markdown_view
            } else {
                DocView::Edit
            },
            preview,
            _subscriptions: subscriptions,
        });
        self.activate(self.docs.len() - 1, window, cx);
        self.load_head(self.docs.len() - 1, cx);
    }

    /// Open `path` with the caret at `row` and byte column `col`.
    fn open_at(
        &mut self,
        path: PathBuf,
        row: u32,
        col: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open(path.clone(), window, cx);
        let Some(doc) = self.docs.iter().find(|doc| doc.path == path) else {
            return;
        };
        doc.editor.update(cx, |editor, cx| {
            let character = editor
                .text()
                .to_string()
                .lines()
                .nth(row as usize)
                .map(|line| {
                    let mut end = (col as usize).min(line.len());
                    while !line.is_char_boundary(end) {
                        end -= 1;
                    }
                    line[..end].encode_utf16().count() as u32
                })
                .unwrap_or(0);
            editor.set_cursor_position(Position::new(row, character), window, cx);
        });
    }

    fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        self.active = Some(ix);
        self.diff_active = false;
        // A preview has no caret; keys go to the window instead.
        if doc.view == DocView::Preview {
            self.focus.focus(window, cx);
        } else {
            doc.editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        let path = doc.path.clone();
        self.tree.update(cx, |tree, cx| tree.select(&path, cx));
        cx.notify();
    }

    fn refresh_dirty(&mut self, editor: &Entity<EditorState>, cx: &mut Context<Self>) {
        let Some(ix) = self.docs.iter().position(|d| &d.editor == editor) else {
            return;
        };
        let doc = &mut self.docs[ix];
        let dirty = *editor.read(cx).text() != doc.saved.as_str();
        if dirty != doc.dirty {
            doc.dirty = dirty;
            cx.notify();
        }
        self.refresh_gutter(ix, cx);
        self.update_preview(ix, cx);
    }

    /// Write the editor's text to disk as it is.
    fn write_doc(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        let text = doc.editor.read(cx).text().to_string();
        let on_disk = if doc.crlf {
            text.replace('\n', "\r\n")
        } else {
            text.clone()
        };
        match std::fs::write(&doc.path, on_disk) {
            Ok(()) => {
                doc.saved = text;
                doc.dirty = false;
                self.refresh_git(cx);
            }
            Err(err) => self.notice = Some(format!("保存失败：{err}").into()),
        }
        cx.notify();
    }

    /// Format the document, then save it when `save` is set.
    ///
    /// Saving formats Go only: a Go file that is not gofmt-clean is never what
    /// anyone wants, while reformatting other files is a deliberate act.
    fn format_doc(&mut self, ix: usize, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let language = doc.language;
        if save && language != "go" {
            self.notice = None;
            self.write_doc(ix, cx);
            return;
        }
        let editor = doc.editor.clone();
        let before = editor.read(cx).text().to_string();
        let dir = doc.path.parent().map(PathBuf::from);
        let formatting = cx.background_executor().spawn({
            let before = before.clone();
            async move { format::format(language, &before, dir.as_deref()) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = formatting.await;
            this.update_in(cx, |this, window, cx| {
                let Some(ix) = this.docs.iter().position(|doc| doc.editor == editor) else {
                    return;
                };
                match result {
                    Ok(formatted) => {
                        this.notice = None;
                        // Typing went on while the formatter ran: its answer is
                        // for text that no longer exists, so leave the text be.
                        let unchanged = *editor.read(cx).text() == before.as_str();
                        if let Some(formatted) = formatted
                            && unchanged
                            && formatted != before
                        {
                            replace_text(&editor, &before, &formatted, window, cx);
                        }
                    }
                    Err(reason) => this.notice = Some(reason.into()),
                }
                if save {
                    this.write_doc(ix, cx);
                } else {
                    this.refresh_dirty(&editor, cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn close(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.docs.len() {
            return;
        }
        // Like GoLand, nothing is lost on close: unsaved edits are written out.
        if self.docs[ix].dirty {
            self.write_doc(ix, cx);
        }
        self.docs.remove(ix);
        if ix < self.diff_slot {
            self.diff_slot -= 1;
        }
        self.active = match self.active {
            _ if self.docs.is_empty() => None,
            Some(active) if active > ix => Some(active - 1),
            Some(active) => Some(active.min(self.docs.len() - 1)),
            None => None,
        };
        match self.active {
            Some(active) => self.activate(active, window, cx),
            None => {
                self.focus.focus(window, cx);
                cx.notify();
            }
        }
    }

    fn set_root(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        for ix in 0..self.docs.len() {
            if self.docs[ix].dirty {
                self.write_doc(ix, cx);
            }
        }
        self.docs.clear();
        self.active = None;
        // Shells started in the old folder do not follow to the new one.
        self.terminals.clear();
        self.terminal_visible = false;
        self.log = None;
        self.diff = None;
        self.diff_active = false;
        self.git = None;
        self.branches.clear();
        self.excluded.clear();
        self.palette = None;
        self.project_files = None;
        // The program of the old project stops with it.
        self.run = run_panel::RunState::new();
        self.recent = crate::recent::record(&root);
        self.root = root.clone();
        self.settings = settings::load(&root);
        if let Ok(mut watched) = self.watched_hidden.write() {
            *watched = self.settings.hidden.clone();
        }
        let hidden = self.settings.hidden.clone();
        self.tree.update(cx, |tree, cx| tree.set_root(root, hidden, cx));
        self.refresh_git(cx);
        self.watch(window, cx);
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// Give the keyboard back to the open file, or to the window without one.
    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active.and_then(|ix| self.docs.get(ix)) {
            Some(doc) => doc.editor.update(cx, |editor, cx| editor.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
    }

    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let view = cx.new(|cx| TerminalView::new(root, cx));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, view, event: &TerminalEvent, window, cx| match event {
                TerminalEvent::OpenFile(path, row, col) => {
                    this.open_reference(path, *row, *col, None, window, cx)
                }
                TerminalEvent::Exited(_) => {
                    if let Some(ix) = this.terminals.iter().position(|tab| &tab.view == view) {
                        this.close_terminal(ix, window, cx);
                    }
                }
            },
        );
        self.terminals_opened += 1;
        self.terminals.push(TerminalTab {
            name: format!("终端 {}", self.terminals_opened).into(),
            view,
            _subscription: subscription,
        });
        self.show_terminal(self.terminals.len() - 1, window, cx);
    }

    fn show_terminal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.terminals.get(ix) else {
            return;
        };
        let view = tab.view.clone();
        self.active_terminal = ix;
        self.terminal_visible = true;
        self.bottom_tab = BottomTab::Terminal;
        // What runs in the terminal reads files from disk, not from the editor.
        self.save_all(cx);
        view.update(cx, |view, cx| view.focus(window, cx));
        cx.notify();
    }

    fn close_terminal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.terminals.len() {
            return;
        }
        self.terminals.remove(ix);
        if self.active_terminal > ix || self.active_terminal >= self.terminals.len() {
            self.active_terminal = self.active_terminal.saturating_sub(1);
        }
        if self.terminals.is_empty() {
            self.terminal_visible = false;
            self.focus_editor(window, cx);
        } else if self.terminal_visible {
            self.show_terminal(self.active_terminal, window, cx);
        }
        cx.notify();
    }

    fn on_toggle_terminal(&mut self, _: &ToggleTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal_visible && self.bottom_tab == BottomTab::Terminal {
            self.terminal_visible = false;
            self.focus_editor(window, cx);
            cx.notify();
        } else if self.terminals.is_empty() {
            self.new_terminal(window, cx);
        } else {
            self.show_terminal(self.active_terminal, window, cx);
        }
    }

    fn on_new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
        self.new_terminal(window, cx);
    }

    pub(super) fn on_show_git_log(&mut self, _: &ShowGitLog, window: &mut Window, cx: &mut Context<Self>) {
        if self.log_visible() {
            self.terminal_visible = false;
            self.focus_editor(window, cx);
            cx.notify();
        } else {
            self.show_log(window, cx);
        }
    }

    fn on_save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.format_doc(ix, true, window, cx);
        }
    }

    fn on_format(&mut self, _: &Format, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.format_doc(ix, false, window, cx);
        }
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.diff_active {
            self.close_diff(window, cx);
        } else if let Some(ix) = self.active {
            self.close(ix, window, cx);
        }
    }

    fn on_toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sidebar_tab(SidebarTab::Files, cx);
    }

    fn on_show_commit(&mut self, _: &ShowCommit, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sidebar_tab(SidebarTab::Changes, cx);
        if self.sidebar_visible {
            self.commit_message
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }

    fn on_toggle_theme(&mut self, _: &ToggleTheme, _: &mut Window, cx: &mut Context<Self>) {
        let mode = if cx.theme().is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        Theme::change(mode, None, cx);
    }

    fn on_open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(root) = paths.into_iter().next()
            {
                this.update_in(cx, |this, window, cx| this.set_root(root, window, cx))
                    .ok();
            }
        })
        .detach();
    }

}

/// Turn the editor's text from `before` into `after` by editing only the
/// lines that differ, so the caret, the scroll position, and undo survive.
fn replace_text(
    editor: &Entity<EditorState>,
    before: &str,
    after: &str,
    window: &mut Window,
    cx: &mut App,
) {
    if before == after {
        return;
    }
    let edits = format::line_edits(before, after);
    editor.update(cx, |editor, cx| {
        let caret = format::map_position(before, after, editor.cursor_position());
        editor.apply_lsp_edits(&edits, window, cx);
        if *editor.text() != after {
            editor.set_value(after.to_string(), window, cx);
        }
        editor.set_cursor_position(caret, window, cx);
    });
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests;
