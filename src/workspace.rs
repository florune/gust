use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::input::{
    EditorState, GoToDefinition, GutterDecorationCollection, InputEvent, Position,
    ShowCompletions, TabSize, TextareaState, highlight_characters,
};
use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::*;

use crate::complete::{GoFile, uri_to_path, usages_target};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::format;
use crate::git;
use crate::goindex::Index;
use crate::lang;
use crate::local_history;
use crate::settings::{self, ProjectSettings};
use crate::terminal::{TerminalEvent, TerminalView};

mod action_search;
mod app_menu;
mod autoscroll;
mod blame;
mod bracket_view;
mod changes;
mod compare_view;
mod file_ops;
mod history_view;
mod intentions;
mod keymap_view;
mod commands;
mod conflict;
mod editing;
mod layout;
mod lens;
mod log;
mod markdown;
mod merge_view;
mod navigation;
mod outline;
mod palette;
mod problems_view;
mod quick_doc;
mod rename_symbol;
mod rollback;
mod run_panel;
mod selection;
mod session;
mod settings_view;
mod split;
mod tabs;
mod test_runner;
mod usages;
mod windows;
mod worktrees_view;

pub use windows::open_window;

/// Bind the shortcuts the user chose in place of the defaults.
pub fn apply_keymap(keymap: &std::collections::BTreeMap<String, Vec<String>>, cx: &mut App) {
    commands::apply_keymap(keymap, &Default::default(), cx);
}

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
    outline::init(cx);
    navigation::init(cx);
    selection::init(cx);
    file_ops::init(cx);
    editing::init(cx);
    problems_view::init(cx);
    rollback::init(cx);
    quick_doc::init(cx);
    test_runner::init(cx);
    rename_symbol::init(cx);
    settings_view::init(cx);
    keymap_view::init(cx);
    app_menu::init(cx);
    intentions::init(cx);
    action_search::init(cx);
    merge_view::init(cx);
    windows::init(cx);
    worktrees_view::init(cx);
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
    /// The declarations of a Go file, read again after edits.
    outline: Rc<Vec<crate::goindex::OutlineItem>>,
    outline_task: Task<()>,
    /// The disk's text, when the file changed there under unsaved edits;
    /// nothing is written until the user picks a side.
    conflict: Option<String>,
    /// Who last changed each line, for the note after the caret's line.
    blame: Rc<crate::history::Blame>,
    blame_task: Task<()>,
    /// How often its declarations are used, for the notes after them.
    lenses: lens::Lenses,
    /// Its brackets, colored by depth.
    brackets: bracket_view::DocBrackets,
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
    /// The right pane, when the editor is split.
    split: Option<split::Split>,
    /// Places the caret was before jumps, for Back; and after, for Forward.
    back: Vec<navigation::Place>,
    forward: Vec<navigation::Place>,
    /// Going back or forward, which is not itself a jump to remember.
    navigating: bool,
    /// The editor went to a definition by itself during the last Ctrl+press.
    jumped: Rc<std::cell::Cell<bool>>,
    /// Where a Ctrl+press started, until its release.
    ctrl_click: Option<Point<Pixels>>,
    /// A press is being sent again with other modifiers.
    remapping: Rc<std::cell::Cell<bool>>,
    /// The steps of Extend Selection, for Shrink Selection.
    expansions: selection::Expansions,
    /// Taps of Shift, for the double tap that opens the file search.
    shift_taps: palette::ShiftTaps,
    /// The box asking for a new name, or whether to delete.
    file_prompt: Option<file_ops::FilePrompt>,
    /// Files shown, the last one first, for Ctrl+E.
    recent_files: Vec<PathBuf>,
    /// Compile errors and vet findings, by package.
    problems: problems_view::Problems,
    /// The committed lines of a change, shown after a click on its mark.
    hunk: Option<rollback::Hunk>,
    /// A project to open, waiting for: this window or a new one?
    project_choice: Option<windows::ProjectChoice>,
    /// The documentation popup of Ctrl+Q.
    quick_doc: Option<quick_doc::QuickDoc>,
    /// Settings of the application, the same for every project.
    app: settings::AppSettings,
    /// The settings page, while it is open.
    settings_page: Option<FocusHandle>,
    /// How far the tab bar is scrolled, for tabs that do not fit.
    tab_scroll: ScrollHandle,
    /// The tab last scrolled into view, so it is done once per change of tab.
    tab_in_view: std::cell::Cell<Option<usize>>,
    tab_bar_width: std::cell::Cell<Pixels>,
    /// The repository's worktrees, and the box that adds one.
    worktrees: worktrees_view::Worktrees,
    /// The shortcuts page, while it is open.
    keymap_page: Option<keymap_view::KeymapPage>,
    /// Bumped when a save or a change on disk may change usage counts.
    lens_generation: u64,
    /// The About box, while it is open.
    about: Option<FocusHandle>,
    /// Wheel movement with Ctrl not yet large enough to change the font size.
    zoom_remainder: f32,
    /// The next commit replaces the last one.
    amend: bool,
    /// The file and caret Alt+Enter was pressed at.
    intention_target: Option<(PathBuf, usize)>,
    /// What had the keyboard when Find Action opened.
    action_focus: Option<FocusHandle>,
    /// The list of conflicted files and the merge window, while open.
    merge_dialog: Option<merge_view::MergeDialog>,
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
    /// What just went through, shown in the status bar for a moment.
    info: Option<SharedString>,
    info_clear: Task<()>,
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
                    FileTreeEvent::New { dir, folder } => this.open_file_prompt(
                        file_ops::FileAction::New {
                            dir: dir.clone(),
                            folder: *folder,
                        },
                        window,
                        cx,
                    ),
                    FileTreeEvent::Rename(path) => {
                        this.open_file_prompt(file_ops::FileAction::Rename(path.clone()), window, cx)
                    }
                    FileTreeEvent::Delete(path) => {
                        this.open_file_prompt(file_ops::FileAction::Delete(path.clone()), window, cx)
                    }
                    FileTreeEvent::History(path) => this.show_local_history(path.clone(), window, cx),
                },
            ),
            // Like GoLand: leaving the window saves, so tools outside it see
            // what the editor shows.
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.remember_session(cx);
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
            split: None,
            back: Vec::new(),
            forward: Vec::new(),
            navigating: false,
            jumped: Rc::default(),
            ctrl_click: None,
            remapping: Rc::default(),
            expansions: Default::default(),
            shift_taps: Default::default(),
            file_prompt: None,
            recent_files: Vec::new(),
            problems: Default::default(),
            hunk: None,
            project_choice: None,
            quick_doc: None,
            app: settings::load_app(),
            settings_page: None,
            keymap_page: None,
            worktrees: Default::default(),
            tab_scroll: ScrollHandle::new(),
            tab_in_view: std::cell::Cell::new(None),
            tab_bar_width: std::cell::Cell::new(px(0.)),
            lens_generation: 0,
            about: None,
            zoom_remainder: 0.,
            amend: false,
            intention_target: None,
            action_focus: None,
            merge_dialog: None,
            project_files: None,
            terminals: Vec::new(),
            active_terminal: 0,
            terminal_visible: false,
            bottom_tab: BottomTab::Terminal,
            log: None,
            terminals_opened: 0,
            memory_mb: 0,
            notice: None,
            info: None,
            info_clear: Task::ready(()),
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
        this.restore_session(window, cx);
        // Closing the window keeps the session for next time.
        cx.on_release(|this, cx| this.remember_session(cx)).detach();
        this
    }

    /// Follow changes made to the folder by other programs.
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

    pub(super) fn files_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_files = None;
        self.lenses_out_of_date(cx);
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
        let editor = doc.editor.clone();
        let mine = editor.read(cx).text().to_string();
        let doc = &mut self.docs[ix];
        if on_disk == doc.saved || on_disk == mine {
            // Back to what was read, or the same as the edits: nothing to decide.
            let settled = doc.conflict.take().is_some();
            doc.saved = on_disk;
            doc.crlf = crlf;
            if settled {
                let path = doc.path.clone();
                self.refresh_dirty(&editor, cx);
                self.refresh_disk_compare(&path, cx);
                cx.notify();
            }
            return;
        }
        local_history::record(&self.root, &doc.path, &doc.saved, "外部修改前");
        if doc.dirty {
            local_history::record(&self.root, &doc.path, &mine, "未保存的修改");
        }
        let doc = &mut self.docs[ix];
        if doc.dirty {
            // Neither side is dropped: the user decides.
            doc.conflict = Some(on_disk);
            doc.crlf = crlf;
            let path = doc.path.clone();
            self.refresh_disk_compare(&path, cx);
        } else {
            replace_text(&editor, &mine, &on_disk, window, cx);
            let doc = &mut self.docs[ix];
            doc.saved = on_disk;
            doc.crlf = crlf;
            self.refresh_dirty(&editor, cx);
        }
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

    /// Write every file with unsaved edits, except those whose disk version
    /// changed under them: those wait for the user's decision.
    fn save_all(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.docs.len() {
            if self.docs[ix].dirty && self.docs[ix].conflict.is_none() {
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
        // With the right pane in use, files open there.
        let to_split = self.split_focused() && !self.diff_active;
        if self.current_place(cx).is_some_and(|here| here.path != path) {
            self.remember_place(cx);
        }
        if let Some(ix) = self.docs.iter().position(|d| d.path == path) {
            if to_split {
                self.split_right(&path, window, cx);
            } else {
                self.activate(ix, window, cx);
            }
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
            self.attach_go(&editor, &path, cx);
        }
        let subscriptions = vec![
            cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| match event {
                InputEvent::Change => this.refresh_dirty(&editor, cx),
                // The left pane is the one in use now.
                InputEvent::Focus => {
                    if let Some(split) = &mut this.split {
                        split.focused = false;
                        cx.notify();
                    }
                }
                _ => {}
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
            outline: Rc::default(),
            outline_task: Task::ready(()),
            conflict: None,
            blame: Rc::default(),
            blame_task: Task::ready(()),
            lenses: Default::default(),
            brackets: Default::default(),
            _subscriptions: subscriptions,
        });
        self.refresh_outline(self.docs.len() - 1, true, cx);
        self.refresh_brackets(self.docs.len() - 1, cx);
        let opened = self.docs[self.docs.len() - 1].path.clone();
        self.check_package(&opened, cx);
        if to_split {
            let path = self.docs[self.docs.len() - 1].path.clone();
            self.split_right(&path, window, cx);
        } else {
            self.activate(self.docs.len() - 1, window, cx);
        }
        self.load_head(self.docs.len() - 1, cx);
    }

    /// Completion, definitions, and usages for an editor of the Go file `path`.
    fn attach_go(&self, editor: &Entity<EditorState>, path: &Path, cx: &mut Context<Self>) {
        let go = Rc::new(GoFile {
            index: self.index.clone(),
            path: path.to_path_buf(),
        });
        let jumped = self.jumped.clone();
        let workspace = cx.entity().downgrade();
        editor.update(cx, |editor, _| {
            let lsp = editor.lsp_mut();
            lsp.completion_provider = Some(go.clone());
            lsp.hover_provider = Some(go.clone());
            lsp.definition_provider = Some(go);
            lsp.completion_menu.max_width = px(560.);
            lsp.set_completion_highlighter(Rc::new(highlight_characters));
            // Definitions may be in another file, which only the workspace can show.
            // A declaration "goes to" its usages instead.
            lsp.show_document = Some(Rc::new(move |params, window, cx| {
                jumped.set(true);
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

    /// Open `path` with the caret at `row` and byte column `col`.
    fn open_at(
        &mut self,
        path: PathBuf,
        row: u32,
        col: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .current_place(cx)
            .is_some_and(|here| here.path == path && here.row != row)
        {
            self.remember_place(cx);
        }
        self.open(path.clone(), window, cx);
        let Some(doc) = self.docs.iter().find(|doc| doc.path == path) else {
            return;
        };
        // The file may have opened in the right pane.
        let editor = match &self.split {
            Some(split) if split.focused && split.path == path => split.editor.clone(),
            _ => doc.editor.clone(),
        };
        editor.update(cx, |editor, cx| {
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
        // The left pane is in use now; focus events only arrive next frame.
        if let Some(split) = &mut self.split {
            split.focused = false;
        }
        // A preview has no caret; keys go to the window instead.
        if doc.view == DocView::Preview {
            self.focus.focus(window, cx);
        } else {
            doc.editor.update(cx, |editor, cx| editor.focus(window, cx));
        }
        let path = doc.path.clone();
        self.note_recent_file(&path);
        self.tree.update(cx, |tree, cx| tree.select(&path, cx));
        cx.notify();
    }

    fn refresh_dirty(&mut self, editor: &Entity<EditorState>, cx: &mut Context<Self>) {
        let Some(ix) = self.docs.iter().position(|d| &d.editor == editor) else {
            return;
        };
        let doc = &mut self.docs[ix];
        // Edits that came to match the disk's version settle a conflict.
        if doc
            .conflict
            .as_ref()
            .is_some_and(|disk| *editor.read(cx).text() == disk.as_str())
        {
            doc.saved = doc.conflict.take().unwrap_or_default();
            cx.notify();
        }
        let dirty = *editor.read(cx).text() != doc.saved.as_str();
        if dirty != doc.dirty {
            doc.dirty = dirty;
            cx.notify();
        }
        self.refresh_gutter(ix, cx);
        self.update_preview(ix, cx);
        self.refresh_outline(ix, false, cx);
        self.refresh_brackets(ix, cx);
        self.refresh_blame(ix, false, cx);
        let path = self.docs[ix].path.clone();
        self.refresh_disk_compare(&path, cx);
    }

    /// Write the editor's text to disk as it is.
    fn write_doc(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        let text = doc.editor.read(cx).text().to_string();
        if doc.saved != text {
            local_history::record(&self.root, &doc.path, &doc.saved, "保存前");
        }
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        let on_disk = if doc.crlf {
            text.replace('\n', "\r\n")
        } else {
            text.clone()
        };
        match std::fs::write(&doc.path, on_disk) {
            Ok(()) => {
                doc.saved = text;
                doc.dirty = false;
                // Saving on purpose keeps these edits over the disk's.
                doc.conflict = None;
                let saved = doc.path.clone();
                let written = doc.saved.clone();
                if doc.language == "go" {
                    self.lenses_out_of_date(cx);
                }
                self.check_package(&saved, cx);
                self.stage_if_resolved(&saved, &written, cx);
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
        // A file in conflict stays open until the user picks a side.
        if self.docs[ix].conflict.is_some() {
            self.activate(ix, window, cx);
            self.notice = Some(format!("{} 存在未解决的冲突", self.docs[ix].title).into());
            cx.notify();
            return;
        }
        // Like GoLand, nothing is lost on close: unsaved edits are written out.
        if self.docs[ix].dirty {
            self.write_doc(ix, cx);
        }
        let removed = self.docs.remove(ix);
        if self.split.as_ref().is_some_and(|split| split.path == removed.path) {
            self.split = None;
        }
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
        self.remember_session(cx);
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
        self.problems = Default::default();
        self.split = None;
        self.back.clear();
        self.forward.clear();
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
        self.recent_files.clear();
        self.restore_session(window, cx);
        cx.notify();
    }

    /// Give the keyboard back to the open file, or to the window without one.
    fn focus_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.active.and_then(|ix| self.docs.get(ix)) {
            Some(doc) => doc.editor.update(cx, |editor, cx| editor.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
    }

    /// Say in the status bar that something went through; it goes away by itself.
    pub(super) fn inform(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.info = Some(text.into());
        self.info_clear = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs(6)).await;
            this.update(cx, |this, cx| {
                this.info = None;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    fn new_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.new_terminal_in(self.root.clone(), window, cx);
    }

    /// A terminal whose shell starts in `dir`.
    pub(super) fn new_terminal_in(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let elsewhere = (dir != self.root).then(|| {
            dir.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
        });
        let view = cx.new(|cx| TerminalView::new(dir, cx));
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
            name: match elsewhere {
                Some(folder) => format!("终端 {} · {folder}", self.terminals_opened).into(),
                None => format!("终端 {}", self.terminals_opened).into(),
            },
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
            if self.app.format_on_save {
                self.format_doc(ix, true, window, cx);
            } else {
                self.write_doc(ix, cx);
            }
        }
    }

    fn on_format(&mut self, _: &Format, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.format_doc(ix, false, window, cx);
        }
    }

    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.split_focused() && !self.diff_active {
            self.close_split(window, cx);
        } else if self.diff_active {
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
        let dark = mode == ThemeMode::Dark;
        self.change_app_settings(cx, |app| app.dark = dark);
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
                this.update_in(cx, |this, window, cx| this.choose_project(root, window, cx))
                    .ok();
            }
        })
        .detach();
    }

}

/// Turn the editor's text from `before` into `after` by editing only the
/// lines that differ, so the caret, the scroll position, and undo survive.
/// The keyboard stays where it is: the editor may be one not in view.
fn replace_text(
    editor: &Entity<EditorState>,
    before: &str,
    after: &str,
    window: &mut Window,
    cx: &mut App,
) {
    use gpui_kit::component::RopeExt as _;
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
        let offset = editor.text().position_to_offset(&caret);
        editor.set_selected_range(offset..offset, cx);
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
