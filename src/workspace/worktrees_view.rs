//! The worktrees panel: every checkout of the repository with what is going
//! on in it, and the box that adds one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use super::changes::SidebarTab;
use crate::settings;
use crate::ui::{UI_SMALL, UI_TEXT, icon};
use crate::worktree::{self, Activity, Merged, Strategy, Worktree};

actions!(gust, [ShowWorktrees, NewWorktree, CloseNewWorktree, CloseMergeWorktree]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseNewWorktree, Some("NewWorktree")),
        KeyBinding::new("escape", CloseMergeWorktree, Some("MergeWorktree")),
    ]);
}

/// How often the panel looks at the worktrees again while it is shown:
/// work in them happens outside this window.
const POLL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct Worktrees {
    pub list: Vec<Worktree>,
    pub activity: HashMap<PathBuf, Activity>,
    /// Marker colors chosen, as [`settings::worktree_colors`] reads them.
    pub colors: HashMap<String, String>,
    polling: Option<Task<()>>,
    pub dialog: Option<NewWorktreeDialog>,
    pub merging: Option<MergeWorktreeDialog>,
    /// A worktree just merged, its branch, and whether that was a squash:
    /// offered for removal.
    pub cleanup: Option<(PathBuf, String, bool)>,
}

/// The box that brings a worktree's branch into the repository's branch.
pub(super) struct MergeWorktreeDialog {
    pub(super) path: PathBuf,
    branch: String,
    base: String,
    pub(super) strategy: Strategy,
    pub(super) message: Entity<InputState>,
    /// Files with uncommitted changes in the worktree.
    changes: usize,
    /// Commit those first, so they are merged too.
    pub(super) commit_first: bool,
    /// The branch's own commits, oldest first.
    subjects: Vec<String>,
    error: Option<SharedString>,
    busy: bool,
    focus: FocusHandle,
}

pub(super) struct NewWorktreeDialog {
    pub(super) branch: Entity<InputState>,
    pub(super) location: Entity<InputState>,
    patterns: Entity<InputState>,
    /// `KEY=VALUE` lines laid over the inherited run configurations.
    pub(super) env: Entity<InputState>,
    base: String,
    /// The location was typed rather than following the branch name.
    location_typed: bool,
    copy_files: bool,
    pub(super) new_window: bool,
    error: Option<SharedString>,
    busy: bool,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// Seconds since the epoch, for [`super::log::when`].
fn seconds(time: std::time::SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs() as i64)
}

impl Workspace {
    /// The repository's own folder: the main worktree.
    fn main_worktree(&self) -> Option<&Worktree> {
        self.worktrees.list.iter().find(|worktree| worktree.main)
    }

    /// The top of the checkout this window shows.
    fn checkout_root(&self) -> PathBuf {
        self.git.as_ref().map_or_else(|| self.root.clone(), |status| status.root.clone())
    }

    /// The worktree this window shows, when it is not the main one.
    pub(super) fn linked_worktree(&self) -> Option<&Worktree> {
        let root = self.checkout_root();
        self.worktrees
            .list
            .iter()
            .find(|worktree| !worktree.main && worktree::same_path(&worktree.path, &root))
    }

    fn is_current(&self, worktree: &Worktree) -> bool {
        worktree::same_path(&worktree.path, &self.checkout_root())
    }

    /// The folder to open for `worktree`: the same subfolder as this window
    /// shows, when the project is a folder inside the repository.
    fn folder_in(&self, worktree: &Worktree) -> PathBuf {
        let root = self.checkout_root();
        match self.root.strip_prefix(&root) {
            Ok(inside) if !inside.as_os_str().is_empty() && worktree.path.join(inside).is_dir() => {
                worktree.path.join(inside)
            }
            _ => worktree.path.clone(),
        }
    }

    /// The branch the others are compared with: that of the main worktree.
    fn base_branch(&self) -> Option<String> {
        self.main_worktree().and_then(|main| main.branch.clone())
    }

    /// The marker color of the worktree at `path`.
    pub(super) fn worktree_color(&self, path: &Path, cx: &App) -> Hsla {
        let colors = crate::theme::palette(cx);
        match settings::worktree_color(&self.worktrees.colors, path).as_str() {
            "green" => colors.green,
            "orange" => colors.orange,
            "purple" => colors.purple,
            "red" => colors.red,
            "cyan" => colors.cyan,
            _ => cx.theme().primary,
        }
    }

    pub(super) fn set_worktree_color(&mut self, path: PathBuf, color: &'static str, cx: &mut Context<Self>) {
        if let Err(reason) = settings::set_worktree_color(&path, color) {
            self.notice = Some(reason.into());
        }
        self.worktrees.colors = settings::worktree_colors();
        cx.notify();
    }

    /// The other worktrees, by name, with their folders: what a file can be compared with.
    /// The other worktrees: name, branch, folder.
    pub(super) fn other_worktrees(&self) -> Vec<(String, String, PathBuf)> {
        self.worktrees
            .list
            .iter()
            .filter(|worktree| !worktree.prunable && !self.is_current(worktree))
            .map(|worktree| {
                let branch = worktree.branch.clone().unwrap_or_else(|| "游离 HEAD".to_string());
                (worktree.name(), branch, worktree.path.clone())
            })
            .collect()
    }

    /// The top of this window's checkout, for paths that mean the same in every worktree.
    pub(super) fn checkout_top(&self) -> PathBuf {
        self.checkout_root()
    }

    /// Compare the open file `path` with the same file in the worktree at `other_root`.
    pub(super) fn compare_with_worktree(
        &mut self,
        path: &Path,
        other_root: &Path,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.checkout_root();
        let Ok(inside) = path.strip_prefix(&root) else {
            return;
        };
        let Some(doc) = self.docs.iter().find(|doc| doc.path == path) else {
            return;
        };
        let source = super::compare_view::Source::Across {
            file: path.to_path_buf(),
            other: other_root.join(inside),
            name,
            mine: doc.editor.read(cx).text().to_string(),
        };
        let entry = super::compare_view::Entry {
            path: inside.to_string_lossy().replace('\\', "/"),
            old_path: None,
            status: 'M',
        };
        self.show_compare(source, vec![entry], 0, window, cx);
    }

    pub(super) fn on_show_worktrees(&mut self, _: &ShowWorktrees, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sidebar_tab(SidebarTab::Worktrees, cx);
        self.watch_worktrees(cx);
    }

    /// Keep the activity of every worktree current while the panel is shown.
    pub(super) fn watch_worktrees(&mut self, cx: &mut Context<Self>) {
        if !(self.sidebar_visible && self.sidebar_tab == SidebarTab::Worktrees) {
            self.worktrees.polling = None;
            return;
        }
        if self.worktrees.polling.is_some() {
            return;
        }
        let root = self.checkout_root();
        self.worktrees.polling = Some(cx.spawn(async move |this, cx| {
            loop {
                let root = root.clone();
                let found = cx
                    .background_executor()
                    .spawn(async move {
                        let list = worktree::list(&root);
                        let base = list.iter().find(|worktree| worktree.main).and_then(|main| main.branch.clone());
                        let activity: HashMap<PathBuf, Activity> = list
                            .iter()
                            .filter(|worktree| !worktree.prunable)
                            .map(|worktree| {
                                let base = base.as_deref().filter(|_| !worktree.main).unwrap_or("HEAD");
                                (worktree.path.clone(), worktree::activity(&worktree.path, base))
                            })
                            .collect();
                        (list, activity, settings::worktree_colors())
                    })
                    .await;
                let shown = this
                    .update(cx, |this, cx| {
                        this.worktrees.list = found.0;
                        this.worktrees.activity = found.1;
                        this.worktrees.colors = found.2;
                        cx.notify();
                        this.sidebar_visible && this.sidebar_tab == SidebarTab::Worktrees
                    })
                    .unwrap_or(false);
                if !shown {
                    this.update(cx, |this, _| this.worktrees.polling = None).ok();
                    break;
                }
                cx.background_executor().timer(POLL).await;
            }
        }));
    }

    pub(super) fn open_worktree(&mut self, path: PathBuf, new_window: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self
            .worktrees
            .list
            .iter()
            .find(|worktree| worktree.path == path)
            .map(|worktree| self.folder_in(worktree))
        else {
            return;
        };
        if new_window {
            cx.defer(move |cx| super::open_window(target, Vec::new(), cx));
        } else {
            self.choose_project(target, window, cx);
        }
    }

    pub(super) fn on_new_worktree(&mut self, _: &NewWorktree, window: &mut Window, cx: &mut Context<Self>) {
        self.open_new_worktree(window, cx);
    }

    pub(super) fn open_new_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(status) = &self.git else {
            self.notice = Some("当前文件夹不是 Git 仓库".into());
            cx.notify();
            return;
        };
        let base = status.branch.clone();
        let patterns = self.app.worktree_files.join(", ");
        let branch = cx.new(|cx| InputState::new(window, cx).placeholder("新分支或已有分支，如 feature/login"));
        let location = cx.new(|cx| InputState::new(window, cx).placeholder("工作树文件夹"));
        let patterns = cx.new(|cx| InputState::new(window, cx).default_value(patterns));
        let env = cx.new(|cx| InputState::new(window, cx).placeholder("如 PORT=8081，多个以分号分隔"));
        let subscriptions = vec![
            cx.subscribe_in(&branch, window, |this, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let typed = input.read(cx).value().to_string();
                    this.branch_typed(typed, window, cx);
                }
                InputEvent::PressEnter { .. } => this.create_worktree(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&location, window, |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    if let Some(dialog) = &mut this.worktrees.dialog {
                        dialog.location_typed = true;
                    }
                }
                InputEvent::PressEnter { .. } => this.create_worktree(window, cx),
                _ => {}
            }),
        ];
        branch.update(cx, |input, cx| input.focus(window, cx));
        self.worktrees.dialog = Some(NewWorktreeDialog {
            branch,
            location,
            patterns,
            env,
            base,
            location_typed: false,
            copy_files: true,
            new_window: self.app.worktree_new_window,
            error: None,
            busy: false,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// The location follows the branch name until it is typed by hand.
    fn branch_typed(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        let main = self.main_worktree().map_or_else(|| self.checkout_root(), |main| main.path.clone());
        let Some(dialog) = &mut self.worktrees.dialog else {
            return;
        };
        dialog.error = None;
        if dialog.location_typed {
            cx.notify();
            return;
        }
        let place = if branch.trim().is_empty() {
            String::new()
        } else {
            worktree::default_location(&main, &branch).to_string_lossy().into_owned()
        };
        // Setting the value raises no change, so it still counts as not typed.
        dialog.location.update(cx, |input, cx| input.set_value(place, window, cx));
        cx.notify();
    }

    pub(super) fn on_close_new_worktree(&mut self, _: &CloseNewWorktree, window: &mut Window, cx: &mut Context<Self>) {
        self.close_new_worktree(window, cx);
    }

    fn close_new_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.worktrees.dialog.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    pub(super) fn create_worktree(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let main = self.main_worktree().map_or_else(|| self.checkout_root(), |main| main.path.clone());
        let Some(dialog) = &mut self.worktrees.dialog else {
            return;
        };
        if dialog.busy {
            return;
        }
        let branch = dialog.branch.read(cx).value().trim().to_string();
        let location = dialog.location.read(cx).value().trim().to_string();
        if branch.is_empty() {
            dialog.error = Some("请填写分支名称".into());
            cx.notify();
            return;
        }
        if location.is_empty() {
            dialog.error = Some("请填写工作树的位置".into());
            cx.notify();
            return;
        }
        let patterns: Vec<String> = dialog
            .patterns
            .read(cx)
            .value()
            .split([',', '，', ' '])
            .map(str::trim)
            .filter(|pattern| !pattern.is_empty())
            .map(str::to_string)
            .collect();
        let (base, copy, new_window) = (dialog.base.clone(), dialog.copy_files, dialog.new_window);
        let env = dialog.env.read(cx).value().replace(['；', ';'], "\n");
        dialog.busy = true;
        dialog.error = None;
        let place = PathBuf::from(&location);
        // This window's own settings when it shows the repository: git may
        // spell the folder differently from how the window was opened.
        let inherited = if worktree::same_path(&main, &self.checkout_root()) {
            Some(self.settings.clone())
        } else {
            None
        };
        // What these were set to is what the next worktree starts with.
        let remembered = patterns.clone();
        self.change_app_settings(cx, |app| {
            if copy {
                app.worktree_files = remembered;
            }
            app.worktree_new_window = new_window;
        });
        cx.notify();
        let creating = cx.background_executor().spawn({
            let place = place.clone();
            async move {
                worktree::add(&main, &place, &branch, &base)?;
                // Run configurations and hidden files come along; open files do not.
                let mut inherited = inherited.unwrap_or_else(|| settings::load(&main));
                inherited.session = Default::default();
                for run in &mut inherited.runs {
                    run.env = crate::run::override_env(&run.env, &env);
                }
                let _ = settings::save(&place, &inherited);
                let copied = if copy {
                    let files = worktree::local_files(&main, &patterns);
                    worktree::copy_files(&main, &place, &files)?
                } else {
                    0
                };
                Ok::<usize, String>(copied)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = creating.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(copied) => {
                        this.worktrees.dialog = None;
                        let name = place.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                        this.inform(
                            if copied > 0 {
                                format!("已创建工作树 {name}，复制了 {copied} 个本地文件")
                            } else {
                                format!("已创建工作树 {name}")
                            },
                            cx,
                        );
                        this.refresh_git(cx);
                        this.worktrees.polling = None;
                        this.watch_worktrees(cx);
                        if new_window {
                            cx.defer(move |cx| super::open_window(place, Vec::new(), cx));
                        }
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut this.worktrees.dialog {
                            dialog.busy = false;
                            dialog.error = Some(error.into());
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Show everything done in the worktree at `path` since its branch left the base.
    pub(super) fn review_worktree(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let Some(base) = self.base_branch() else {
            self.notice = Some("主仓库没有检出分支，无法比较".into());
            cx.notify();
            return;
        };
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let reading = cx.background_executor().spawn({
            let (path, base) = (path.clone(), base.clone());
            async move {
                let fork = worktree::fork_point(&path, &base)?;
                let files = worktree::changed_since(&path, &fork);
                Some((fork, files))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let found = reading.await;
            this.update_in(cx, |this, window, cx| {
                let Some((fork, files)) = found else {
                    this.notice = Some(format!("找不到 {name} 与 {base} 的共同提交").into());
                    cx.notify();
                    return;
                };
                if files.is_empty() {
                    this.inform(format!("工作树 {name} 相对 {base} 没有改动"), cx);
                    return;
                }
                let entries = files.iter().map(super::compare_view::Entry::from).collect();
                let source = super::compare_view::Source::Worktree { dir: path, fork, base, name };
                this.show_compare(source, entries, 0, window, cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn open_merge_worktree(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(base), Some(main)) = (self.base_branch(), self.main_worktree().map(|main| main.path.clone())) else {
            self.notice = Some("主仓库没有检出分支，无法合并".into());
            cx.notify();
            return;
        };
        let Some(branch) = self
            .worktrees
            .list
            .iter()
            .find(|worktree| worktree::same_path(&worktree.path, &path))
            .and_then(|worktree| worktree.branch.clone())
        else {
            self.notice = Some("工作树没有检出分支，无法合并".into());
            cx.notify();
            return;
        };
        let subjects = worktree::subjects(&main, &base, &branch);
        let changes = worktree::activity(&path, &base).changes;
        let message = match subjects.as_slice() {
            [only] => only.clone(),
            _ => format!("合并 {branch}"),
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(message));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.worktrees.merging = Some(MergeWorktreeDialog {
            path,
            branch,
            base,
            strategy: Strategy::Squash,
            message: input,
            changes,
            commit_first: changes > 0,
            subjects,
            error: None,
            busy: false,
            focus,
        });
        cx.notify();
    }

    pub(super) fn on_close_merge_worktree(&mut self, _: &CloseMergeWorktree, window: &mut Window, cx: &mut Context<Self>) {
        self.close_merge_worktree(window, cx);
    }

    fn close_merge_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.worktrees.merging.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    pub(super) fn merge_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(main) = self.main_worktree().map(|main| main.path.clone()) else {
            return;
        };
        let here_is_main = worktree::same_path(&main, &self.checkout_root());
        let Some(dialog) = &mut self.worktrees.merging else {
            return;
        };
        if dialog.busy {
            return;
        }
        let message = dialog.message.read(cx).value().trim().to_string();
        if message.is_empty() && dialog.strategy != Strategy::Rebase {
            dialog.error = Some("请填写提交说明".into());
            cx.notify();
            return;
        }
        if dialog.changes > 0 && !dialog.commit_first {
            dialog.error = Some(format!("工作树中有 {} 个文件未提交；请勾选先提交，或先在工作树中处理", dialog.changes).into());
            cx.notify();
            return;
        }
        dialog.busy = true;
        dialog.error = None;
        let (path, branch, base, strategy) = (dialog.path.clone(), dialog.branch.clone(), dialog.base.clone(), dialog.strategy);
        let wip = (dialog.changes > 0).then(|| format!("{branch}：未提交的改动"));
        cx.notify();
        let merging = cx.background_executor().spawn({
            let (path, branch, message) = (path.clone(), branch.clone(), message.clone());
            async move { worktree::merge(&main, &path, &branch, strategy, &message, wip.as_deref()) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = merging.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(Merged::Done) => {
                        this.worktrees.merging = None;
                        this.inform(format!("已将 {branch} 合并到 {base}"), cx);
                        // Its review shows work that is now on the base.
                        let reviewed = this.diff.as_ref().is_some_and(|tab| {
                            matches!(&tab.source, super::compare_view::Source::Worktree { dir, .. } if *dir == path)
                        });
                        if reviewed {
                            this.close_diff(window, cx);
                        }
                        this.worktrees.cleanup = Some((path, branch, strategy == Strategy::Squash));
                        this.offer_cleanup(window, cx);
                    }
                    Ok(Merged::Conflicts) => {
                        this.worktrees.merging = None;
                        if here_is_main {
                            this.notice = Some("合并有冲突：在提交面板中解决冲突后提交".into());
                            this.sidebar_visible = true;
                            this.sidebar_tab = SidebarTab::Changes;
                            // The message waits in the commit box for when the conflicts are settled.
                            this.commit_message.update(cx, |input, cx| input.set_value(message, window, cx));
                        } else {
                            this.notice = Some("合并有冲突：请在主仓库的窗口中，从提交面板解决冲突后提交".into());
                        }
                    }
                    Err(error) => {
                        if let Some(dialog) = &mut this.worktrees.merging {
                            dialog.busy = false;
                            dialog.error = Some(error.into());
                        }
                    }
                }
                this.refresh_git(cx);
                this.worktrees.polling = None;
                this.watch_worktrees(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// After a merge, offer to take the worktree and its branch away.
    pub(super) fn offer_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_prompt.is_some() {
            return;
        }
        if let Some((path, branch, _)) = self.worktrees.cleanup.clone() {
            self.open_file_prompt(super::file_ops::FileAction::CleanupWorktree(path, branch), window, cx);
        }
    }

    /// Take a merged worktree and its branch away.
    pub(super) fn clean_up_worktree(&mut self, path: &Path, branch: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let main = self.main_worktree().ok_or("找不到主仓库")?.path.clone();
        let force_branch = self.worktrees.cleanup.as_ref().is_some_and(|(_, _, squashed)| *squashed);
        self.worktrees.cleanup = None;
        let (path, branch) = (path.to_path_buf(), branch.to_string());
        let removing = cx.background_executor().spawn(async move {
            worktree::remove(&main, &path, false)?;
            worktree::delete_branch(&main, &branch, force_branch)?;
            Ok::<String, String>(branch)
        });
        cx.spawn(async move |this, cx| {
            let result = removing.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(branch) => this.inform(format!("已删除工作树和分支 {branch}"), cx),
                    Err(error) => this.notice = Some(format!("清理时出错：{error}").into()),
                }
                this.refresh_git(cx);
                this.worktrees.polling = None;
                this.watch_worktrees(cx);
            })
            .ok();
        })
        .detach();
        Ok(())
    }

    pub(super) fn render_merge_worktree(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.worktrees.merging.as_ref()?;
        let theme = cx.theme();
        let choice = |id: &'static str, strategy: Strategy, title: &'static str, hint: &'static str| {
            let chosen = dialog.strategy == strategy;
            h_flex()
                .id(id)
                .items_start()
                .gap_2()
                .p_2()
                .rounded(theme.radius)
                .border_1()
                .border_color(if chosen { theme.ring } else { theme.border })
                .when(chosen, |row| row.bg(theme.accent))
                .cursor_pointer()
                .child(
                    div()
                        .mt(px(3.))
                        .size(px(12.))
                        .rounded_full()
                        .border_1()
                        .border_color(if chosen { theme.primary } else { theme.muted_foreground })
                        .when(chosen, |dot| dot.border_3()),
                )
                .child(
                    v_flex()
                        .child(div().text_size(UI_TEXT).child(title))
                        .child(div().text_size(UI_SMALL).text_color(theme.muted_foreground).child(hint)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(dialog) = &mut this.worktrees.merging {
                        dialog.strategy = strategy;
                        cx.notify();
                    }
                }))
        };
        let commits = match dialog.subjects.len() {
            0 => "分支上还没有提交".to_string(),
            n => format!("{n} 个提交：{}", dialog.subjects.join("；")),
        };
        let panel = v_flex()
            .id("merge-worktree")
            .key_context("MergeWorktree")
            .track_focus(&dialog.focus)
            .occlude()
            .w(px(540.))
            .p_4()
            .gap_2()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .text_size(UI_TEXT)
            .on_action(cx.listener(Self::on_close_merge_worktree))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("将 {} 合并到 {}", dialog.branch, dialog.base)))
            .child(div().text_size(UI_SMALL).text_color(theme.muted_foreground).truncate().child(commits))
            .child(choice("merge-squash", Strategy::Squash, "压缩为一个提交", "分支上的全部改动作为一个新提交加到主分支，适合零碎的提交"))
            .child(choice("merge-commit", Strategy::Merge, "合并提交", "保留分支上的每个提交，并新建一个合并提交"))
            .child(choice("merge-rebase", Strategy::Rebase, "变基后快进", "把分支上的提交接到主分支末尾，历史保持一条直线"))
            .when(dialog.strategy != Strategy::Rebase, |panel| {
                panel
                    .child(div().pt_1().text_size(UI_SMALL).text_color(theme.muted_foreground).child("提交说明"))
                    .child(Input::new(&dialog.message).small())
            })
            .when(dialog.changes > 0, |panel| {
                panel.child(
                    Checkbox::new("merge-commit-first")
                        .checked(dialog.commit_first)
                        .label(format!("先提交工作树中 {} 个未提交的文件", dialog.changes))
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if let Some(dialog) = &mut this.worktrees.merging {
                                dialog.commit_first = *checked;
                                cx.notify();
                            }
                        })),
                )
            })
            .when_some(dialog.error.clone(), |panel, error| {
                panel.child(div().text_size(UI_SMALL).text_color(theme.danger).child(error))
            })
            .child(
                h_flex()
                    .pt_1()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("merge-worktree-cancel")
                            .small()
                            .label("取消")
                            .on_click(cx.listener(|this, _, window, cx| this.close_merge_worktree(window, cx))),
                    )
                    .child(
                        Button::new("merge-worktree-ok")
                            .small()
                            .primary()
                            .label(if dialog.busy { "正在合并…" } else { "合并" })
                            .disabled(dialog.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.merge_worktree(window, cx))),
                    ),
            );
        Some(
            div()
                .id("merge-worktree-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(96.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_merge_worktree(window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    /// Ask before removing the worktree at `path`.
    pub(super) fn ask_remove_worktree(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_prompt(super::file_ops::FileAction::RemoveWorktree(path), window, cx);
    }

    /// What the box asking to remove the worktree at `path` says.
    pub(super) fn remove_worktree_detail(&self, path: &Path) -> String {
        let worktree = self.worktrees.list.iter().find(|worktree| worktree.path == path);
        let mut detail = format!("将删除文件夹 {}。", path.display());
        if let Some(branch) = worktree.and_then(|worktree| worktree.branch.as_ref()) {
            detail.push_str(&format!("分支 {branch} 会保留。"));
        }
        let changes = self.worktrees.activity.get(path).map_or(0, |activity| activity.changes);
        if changes > 0 {
            detail.push_str(&format!("其中 {changes} 个文件有未提交的改动，将一并丢失。"));
        }
        detail
    }

    pub(super) fn remove_worktree(&mut self, path: &Path, cx: &mut Context<Self>) -> Result<(), String> {
        let main = self.main_worktree().ok_or("找不到主仓库")?.path.clone();
        if worktree::same_path(path, &self.checkout_root()) {
            return Err("不能删除当前窗口正在使用的工作树".into());
        }
        // Uncommitted changes go too: the box said so.
        let force = self.worktrees.activity.get(path).is_none_or(|activity| activity.changes > 0);
        let path = path.to_path_buf();
        let removing = cx
            .background_executor()
            .spawn(async move { worktree::remove(&main, &path, force).map(|()| path) });
        cx.spawn(async move |this, cx| {
            let result = removing.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(path) => {
                        let name = path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
                        this.inform(format!("已删除工作树 {name}，分支已保留"), cx)
                    }
                    Err(error) => this.notice = Some(format!("无法删除工作树：{error}").into()),
                }
                this.worktrees.polling = None;
                this.watch_worktrees(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        Ok(())
    }

    /// In the title bar, when this window shows a worktree rather than the
    /// repository itself, so edits do not land in the wrong copy unnoticed.
    pub(super) fn render_worktree_marker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let worktree = self.linked_worktree()?;
        let color = self.worktree_color(&worktree.path, cx);
        let theme = cx.theme();
        let main = self.main_worktree().map(|main| main.path.display().to_string()).unwrap_or_default();
        let tip: SharedString = format!("工作树 {} · 主仓库 {main}", worktree.path.display()).into();
        Some(
            h_flex()
                .id("worktree-marker")
                .h(px(22.))
                .px_1p5()
                .gap_1()
                .rounded(theme.radius)
                .text_size(UI_SMALL)
                .text_color(color)
                .bg(color.opacity(0.14))
                .cursor_pointer()
                .child(icon("git-fork").xsmall())
                .child(format!("工作树 · {}", worktree.name()))
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
                .on_click(cx.listener(|this, _, _, cx| {
                    if !(this.sidebar_visible && this.sidebar_tab == SidebarTab::Worktrees) {
                        this.toggle_sidebar_tab(SidebarTab::Worktrees, cx);
                    }
                }))
                .into_any_element(),
        )
    }

    pub(super) fn render_worktrees(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        if self.git.is_none() {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("当前文件夹不是 Git 仓库")
                .into_any_element();
        }
        let base = self.base_branch();
        let workspace = cx.entity().downgrade();
        let rows = self.worktrees.list.iter().enumerate().map(|(ix, worktree)| {
            let current = self.is_current(worktree);
            let activity = self.worktrees.activity.get(&worktree.path);
            let branch = worktree.branch.clone().unwrap_or_else(|| "游离 HEAD".to_string());
            let mut facts: Vec<String> = Vec::new();
            if worktree.prunable {
                facts.push("文件夹已不存在".to_string());
            } else if let Some(activity) = activity {
                if activity.changes > 0 {
                    facts.push(format!("{} 个改动", activity.changes));
                }
                if !worktree.main && activity.fresh && activity.changes == 0 {
                    facts.push("还没有改动".to_string());
                }
                if !worktree.main {
                    if activity.ahead > 0 {
                        facts.push(format!("领先 {}", activity.ahead));
                    }
                    if activity.behind > 0 {
                        facts.push(format!("落后 {}", activity.behind));
                    }
                    if activity.merged()
                        && let Some(base) = &base
                    {
                        facts.push(format!("已合并到 {base}"));
                    }
                }
                if let Some(last) = activity.last {
                    facts.push(super::log::when(seconds(last)));
                }
            }
            let merged = !worktree.main && activity.is_some_and(Activity::merged);
            let marker = self.worktree_color(&worktree.path, cx);
            let path = worktree.path.clone();
            let open_path = worktree.path.clone();
            let window_path = worktree.path.clone();
            let main = worktree.main;
            let badge = |text: &'static str, color: Hsla| {
                div()
                    .px_1()
                    .rounded(px(3.))
                    .text_size(px(10.))
                    .text_color(color)
                    .bg(color.opacity(0.15))
                    .child(text)
            };
            h_flex()
                .id(("worktree", ix))
                .w_full()
                .px_2()
                .py_1p5()
                .gap_2()
                .items_start()
                .when(current, |row| row.bg(theme.list_active))
                .hover(|row| row.bg(theme.list_hover))
                .child(
                    div()
                        .pt_0p5()
                        .text_color(if main { theme.muted_foreground } else { marker })
                        .child(icon(if main { "folder-git-2" } else { "git-fork" }).small()),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            h_flex()
                                .gap_1p5()
                                .text_size(UI_TEXT)
                                .child(div().truncate().child(worktree.name()))
                                .when(main, |row| row.child(badge("主仓库", theme.muted_foreground)))
                                .when(current, |row| row.child(badge("当前", theme.primary)))
                                .when(merged, |row| row.child(badge("可删除", theme.success))),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(UI_SMALL)
                                .text_color(theme.muted_foreground)
                                .child(if facts.is_empty() {
                                    branch.clone()
                                } else {
                                    format!("{branch} · {}", facts.join(" · "))
                                }),
                        ),
                )
                .when(!main, |row| {
                    let review_path = worktree.path.clone();
                    row.child(
                        Self::header_button(
                            SharedString::from(format!("worktree-review-{ix}")),
                            "git-compare",
                            "审阅改动",
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            // The row under the button opens the worktree.
                            cx.stop_propagation();
                            this.review_worktree(review_path.clone(), window, cx)
                        })),
                    )
                })
                .when(!current, |row| {
                    row.child(
                        Self::header_button(
                            SharedString::from(format!("worktree-window-{ix}")),
                            "external-link",
                            "在新窗口中打开",
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.open_worktree(window_path.clone(), true, window, cx)
                        })),
                    )
                })
                .when(!current, |row| {
                    row.on_click(cx.listener(move |this, _, window, cx| {
                        this.open_worktree(open_path.clone(), false, window, cx)
                    }))
                })
                .context_menu({
                    let workspace = workspace.clone();
                    let base_for_menu = base.clone();
                    move |menu, _, _| {
                        let action = |path: PathBuf, run: fn(&mut Workspace, PathBuf, &mut Window, &mut Context<Workspace>)| {
                            let workspace = workspace.clone();
                            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                                workspace.update(cx, |this, cx| run(this, path.clone(), window, cx)).ok();
                            }
                        };
                        let copied = path.to_string_lossy().into_owned();
                        let shown = path.clone();
                        let mut menu = menu;
                        if !current {
                            menu = menu
                                .item(PopupMenuItem::new("在当前窗口打开").on_click(action(path.clone(), |this, path, window, cx| {
                                    this.open_worktree(path, false, window, cx)
                                })))
                                .item(PopupMenuItem::new("在新窗口打开").on_click(action(path.clone(), |this, path, window, cx| {
                                    this.open_worktree(path, true, window, cx)
                                })));
                        }
                        if !main && let Some(base) = &base_for_menu {
                            menu = menu
                                .item(PopupMenuItem::new("审阅改动").on_click(action(path.clone(), |this, path, window, cx| {
                                    this.review_worktree(path, window, cx)
                                })))
                                .item(PopupMenuItem::new(format!("合并到 {base}…")).on_click(action(path.clone(), |this, path, window, cx| {
                                    this.open_merge_worktree(path, window, cx)
                                })))
                                .separator();
                        }
                        menu = menu
                            .item(PopupMenuItem::new("在终端中打开").on_click(action(path.clone(), |this, path, window, cx| {
                                this.new_terminal_in(path, window, cx)
                            })))
                            .separator()
                            .item(PopupMenuItem::new("复制路径").on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                            }))
                            .item(PopupMenuItem::new("在资源管理器中显示").on_click(move |_, _, _| {
                                let _ = crate::proc::command("explorer").arg(&shown).spawn();
                            }));
                        if !main {
                            menu = menu.separator().label("标记颜色");
                            for (label, color) in [
                                ("蓝色", ""),
                                ("绿色", "green"),
                                ("橙色", "orange"),
                                ("紫色", "purple"),
                                ("红色", "red"),
                                ("青色", "cyan"),
                            ] {
                                let workspace = workspace.clone();
                                let path = path.clone();
                                menu = menu.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                                    workspace.update(cx, |this, cx| this.set_worktree_color(path.clone(), color, cx)).ok();
                                }));
                            }
                        }
                        if !main && !current {
                            menu = menu.separator().item(PopupMenuItem::new("删除工作树…").on_click(action(
                                path.clone(),
                                |this, path, window, cx| this.ask_remove_worktree(path, window, cx),
                            )));
                        }
                        menu
                    }
                })
        });
        let loading = self.worktrees.list.is_empty();
        v_flex()
            .id("worktrees")
            .size_full()
            .overflow_y_scroll()
            .children(rows)
            .when(loading, |list| {
                list.child(
                    div()
                        .p_3()
                        .text_size(UI_SMALL)
                        .text_color(theme.muted_foreground)
                        .child("正在读取工作树…"),
                )
            })
            .when(!loading && self.worktrees.list.len() == 1, |list| {
                list.child(
                    div()
                        .p_3()
                        .text_size(UI_SMALL)
                        .text_color(theme.muted_foreground)
                        .child("工作树让同一个仓库同时检出多个分支，各在自己的文件夹里，互不影响。点击右上角的 + 新建一个。"),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_new_worktree(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.worktrees.dialog.as_ref()?;
        let theme = cx.theme();
        let typed = dialog.branch.read(cx).value().trim().to_string();
        let existing = !typed.is_empty() && self.branches.contains(&typed);
        let workspace = cx.entity().downgrade();
        let branches = self.branches.clone();
        let label = |text: &'static str| div().text_size(UI_SMALL).text_color(theme.muted_foreground).child(text);
        let base_picker = Button::new("worktree-base")
            .xsmall()
            .ghost()
            .label(dialog.base.clone())
            .dropdown_caret(true)
            .disabled(existing)
            .dropdown_menu(move |menu, window, _| {
                let mut menu = Self::long_menu(menu, window);
                for branch in &branches {
                    let workspace = workspace.clone();
                    let picked = branch.clone();
                    menu = menu.item(PopupMenuItem::new(branch.clone()).on_click(move |_, _, cx| {
                        workspace
                            .update(cx, |this, cx| {
                                if let Some(dialog) = &mut this.worktrees.dialog {
                                    dialog.base = picked.clone();
                                    cx.notify();
                                }
                            })
                            .ok();
                    }));
                }
                menu
            });
        let panel = v_flex()
            .id("new-worktree")
            .key_context("NewWorktree")
            .track_focus(&dialog.focus)
            .occlude()
            .w(px(540.))
            .p_4()
            .gap_2()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .text_size(UI_TEXT)
            .on_action(cx.listener(Self::on_close_new_worktree))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().pb_1().font_weight(FontWeight::SEMIBOLD).child("新建工作树"))
            .child(label("分支"))
            .child(Input::new(&dialog.branch).small())
            .child(
                h_flex()
                    .gap_1()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .when(existing, |row| row.child(format!("检出已有分支 {typed}")))
                    .when(!existing, |row| row.child("新建分支，基于").child(base_picker)),
            )
            .child(label("位置"))
            .child(Input::new(&dialog.location).small())
            .child(label("运行时的环境变量"))
            .child(Input::new(&dialog.env).small())
            .child(label("加到继承来的每个运行配置上，同名变量以这里为准；同时运行多个工作树时，可用它错开端口。"))
            .child(
                Checkbox::new("worktree-copy")
                    .checked(dialog.copy_files)
                    .label("复制本地文件")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        if let Some(dialog) = &mut this.worktrees.dialog {
                            dialog.copy_files = *checked;
                            cx.notify();
                        }
                    })),
            )
            .when(dialog.copy_files, |panel| {
                panel
                    .child(Input::new(&dialog.patterns).small())
                    .child(label("从主仓库复制 Git 不跟踪、文件名匹配这些模式的文件，如 .env；* 匹配任意字符，以逗号分隔。"))
            })
            .child(
                Checkbox::new("worktree-window")
                    .checked(dialog.new_window)
                    .label("创建后在新窗口中打开")
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        if let Some(dialog) = &mut this.worktrees.dialog {
                            dialog.new_window = *checked;
                            cx.notify();
                        }
                    })),
            )
            .when_some(dialog.error.clone(), |panel, error| {
                panel.child(div().text_size(UI_SMALL).text_color(theme.danger).child(error))
            })
            .child(
                h_flex()
                    .pt_1()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("worktree-cancel")
                            .small()
                            .label("取消")
                            .on_click(cx.listener(|this, _, window, cx| this.close_new_worktree(window, cx))),
                    )
                    .child(
                        Button::new("worktree-create")
                            .small()
                            .primary()
                            .label(if dialog.busy { "正在创建…" } else { "创建" })
                            .disabled(dialog.busy)
                            .on_click(cx.listener(|this, _, window, cx| this.create_worktree(window, cx))),
                    ),
            );
        Some(
            div()
                .id("new-worktree-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(96.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_new_worktree(window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
