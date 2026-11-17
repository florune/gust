//! New, rename, and delete from the file tree. A small box asks for the name
//! (or for confirmation); deleting moves to the Recycle Bin.

use std::path::{Path, PathBuf};

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::ui::{UI_SMALL, UI_TEXT};

actions!(gust, [ConfirmFilePrompt, CancelFilePrompt]);

pub(super) fn init(cx: &mut App) {
    let prompt = Some("FilePrompt");
    cx.bind_keys([
        KeyBinding::new("escape", CancelFilePrompt, prompt),
        KeyBinding::new("enter", ConfirmFilePrompt, prompt),
    ]);
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FileAction {
    /// A file, or with `folder` a folder, in `dir`.
    New { dir: PathBuf, folder: bool },
    Rename(PathBuf),
    Delete(PathBuf),
    /// Take a changed file, from the repository root, back to the last commit.
    Rollback(String),
    /// Rename a Go symbol everywhere it is used.
    RenameSymbol(std::sync::Arc<super::rename_symbol::RenamePlan>),
    /// Make a branch from what is checked out, and switch to it.
    NewBranch,
    /// Put the uncommitted changes aside, with a message.
    Stash,
    /// Take a worktree of the repository away.
    RemoveWorktree(PathBuf),
    /// After a merge: take the worktree and its branch away.
    CleanupWorktree(PathBuf, String),
}

pub(super) struct FilePrompt {
    pub action: FileAction,
    pub input: Entity<InputState>,
    pub error: Option<SharedString>,
    focus: FocusHandle,
    _subscription: Subscription,
}

/// The package a new Go file in `dir` belongs to: that of the files already
/// there, or the folder's name (`main` at the top of a module).
fn package_for(dir: &Path) -> String {
    let existing = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".go"))
        .find_map(|entry| {
            let text = std::fs::read_to_string(entry.path()).ok()?;
            text.lines()
                .map(str::trim)
                .find_map(|line| line.strip_prefix("package "))
                .map(|name| name.trim().trim_end_matches("_test").to_string())
        });
    if let Some(name) = existing {
        return name;
    }
    if dir.join("go.mod").is_file() {
        return "main".to_string();
    }
    let name: String = dir
        .file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .filter(|ch| ch.is_alphanumeric() || *ch == '_')
        .collect();
    if name.is_empty() || name.starts_with(|ch: char| ch.is_ascii_digit()) {
        "main".to_string()
    } else {
        name
    }
}

/// A name typed in the box, as a path below `dir`; `None` when it is not one.
fn below(dir: &Path, typed: &str) -> Option<PathBuf> {
    let typed = typed.trim().replace('\\', "/");
    if typed.is_empty()
        || typed.starts_with('/')
        || typed.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || typed.contains(':')
    {
        return None;
    }
    Some(dir.join(typed))
}

impl Workspace {
    pub(super) fn open_file_prompt(&mut self, action: FileAction, window: &mut Window, cx: &mut Context<Self>) {
        let initial = match &action {
            FileAction::Rename(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            FileAction::RenameSymbol(plan) => plan.name.clone(),
            _ => String::new(),
        };
        let placeholder = match &action {
            FileAction::New { folder: false, .. } => "文件名",
            FileAction::New { folder: true, .. } => "文件夹名",
            FileAction::NewBranch => "分支名称",
            FileAction::Stash => "说明（可选）",
            _ => "",
        };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial.clone())
        });
        // Renaming selects the name, without the extension of a file.
        let selected = match &action {
            FileAction::RenameSymbol(_) => Some(initial.len()),
            _ => initial.rfind('.').filter(|dot| *dot > 0),
        };
        if let Some(end) = selected {
            input.update(cx, |input, cx| input.set_selected_range(0..end, cx));
        }
        let subscription = cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.confirm_file_prompt(window, cx);
            }
        });
        let focus = cx.focus_handle();
        if matches!(action, FileAction::Delete(_) | FileAction::Rollback(_) | FileAction::RemoveWorktree(_) | FileAction::CleanupWorktree(..)) {
            focus.focus(window, cx);
        } else {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        self.file_prompt = Some(FilePrompt {
            action,
            input,
            error: None,
            focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn on_confirm_file_prompt(
        &mut self,
        _: &ConfirmFilePrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_file_prompt(window, cx);
    }

    pub(super) fn on_cancel_file_prompt(
        &mut self,
        _: &CancelFilePrompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_file_prompt(window, cx);
    }

    fn close_file_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_prompt.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    fn confirm_file_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = &self.file_prompt else {
            return;
        };
        let typed = prompt.input.read(cx).value().to_string();
        let result = match prompt.action.clone() {
            FileAction::New { dir, folder } => self.create(&dir, &typed, folder, window, cx),
            FileAction::Rename(path) => self.rename(&path, &typed, cx),
            FileAction::Delete(path) => self.delete(&path, window, cx),
            FileAction::RemoveWorktree(path) => self.remove_worktree(&path, cx),
            FileAction::CleanupWorktree(path, branch) => self.clean_up_worktree(&path, &branch, cx),
            FileAction::Rollback(path) => self.rollback_file(&path, window, cx),
            FileAction::RenameSymbol(plan) => self.apply_rename(&plan, &typed, window, cx),
            FileAction::NewBranch => {
                if typed.trim().is_empty() {
                    Err("请填写分支名称".to_string())
                } else {
                    self.create_branch(typed.trim().to_string(), window, cx);
                    Ok(())
                }
            }
            FileAction::Stash => {
                self.stash_changes(typed, window, cx);
                Ok(())
            }
        };
        match result {
            Ok(()) => {
                self.close_file_prompt(window, cx);
                self.project_files = None;
                self.tree.update(cx, |tree, cx| {
                    tree.reload();
                    cx.notify();
                });
                self.refresh_git(cx);
            }
            Err(error) => {
                if let Some(prompt) = &mut self.file_prompt {
                    prompt.error = Some(error.into());
                }
                cx.notify();
            }
        }
    }

    fn create(
        &mut self,
        dir: &Path,
        typed: &str,
        folder: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let path = below(dir, typed).ok_or("名称无效")?;
        if path.exists() {
            return Err("名称已存在".into());
        }
        if folder {
            return std::fs::create_dir_all(&path).map_err(|err| format!("无法创建：{err}"));
        }
        let parent = path.parent().unwrap_or(dir);
        std::fs::create_dir_all(parent).map_err(|err| format!("无法创建：{err}"))?;
        let content = if path.extension().is_some_and(|ext| ext == "go") {
            format!("package {}\n\n", package_for(parent))
        } else {
            String::new()
        };
        std::fs::write(&path, content).map_err(|err| format!("无法创建：{err}"))?;
        self.open(path.clone(), window, cx);
        // The caret after the package line, ready to type.
        if path.extension().is_some_and(|ext| ext == "go") {
            self.open_at(path, 2, 0, window, cx);
        }
        Ok(())
    }

    fn rename(&mut self, from: &Path, typed: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let dir = from.parent().ok_or("无法重命名项目根目录")?;
        let to = below(dir, typed).ok_or("名称无效")?;
        if to == from {
            return Ok(());
        }
        // A different case of the same name is the same file on Windows.
        let same_file = to.to_string_lossy().eq_ignore_ascii_case(&from.to_string_lossy());
        if to.exists() && !same_file {
            return Err("名称已存在".into());
        }
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|err| format!("无法重命名：{err}"))?;
        }
        std::fs::rename(from, &to).map_err(|err| format!("无法重命名：{err}"))?;
        // Open files follow, folders' contents too.
        for doc in &mut self.docs {
            if let Ok(rest) = doc.path.strip_prefix(from) {
                doc.path = if rest.as_os_str().is_empty() {
                    to.clone()
                } else {
                    to.join(rest)
                };
                doc.title = doc
                    .path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
                    .into();
            }
        }
        if let Some(split) = &mut self.split
            && let Ok(rest) = split.path.strip_prefix(from)
        {
            split.path = if rest.as_os_str().is_empty() { to.clone() } else { to.join(rest) };
        }
        cx.notify();
        Ok(())
    }

    fn delete(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        // Tests delete outright rather than fill the user's Recycle Bin.
        let deleted = if cfg!(test) {
            if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            }
            .map_err(|err| err.to_string())
        } else {
            trash::delete(path).map_err(|err| err.to_string())
        };
        deleted.map_err(|err| format!("无法删除：{err}"))?;
        // What was open from there goes away, unsaved edits and all.
        while let Some(ix) = self.docs.iter().position(|doc| doc.path.starts_with(path)) {
            self.docs[ix].dirty = false;
            self.docs[ix].conflict = None;
            self.close(ix, window, cx);
        }
        Ok(())
    }

    pub(super) fn render_file_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = self.file_prompt.as_ref()?;
        let theme = cx.theme();
        let relative = |path: &Path| super::relative_path(&self.root, path);
        let (title, detail, delete) = match &prompt.action {
            FileAction::New { dir, folder } => {
                let place = relative(dir);
                (
                    if *folder { "新建文件夹" } else { "新建文件" },
                    format!("位置：{}", if place.is_empty() { "项目根目录".to_string() } else { place }),
                    false,
                )
            }
            FileAction::Rename(path) => ("重命名", relative(path), false),
            FileAction::Delete(path) => {
                let what = if path.is_dir() { "文件夹及其中所有内容" } else { "文件" };
                ("删除", format!("将{what} {} 移到回收站。", relative(path)), true)
            }
            FileAction::NewBranch => (
                "新建分支",
                format!("基于 {}", self.git.as_ref().map_or(String::new(), |status| status.branch.clone())),
                false,
            ),
            FileAction::Stash => ("暂存改动", "未提交的修改与新文件将被暂存，工作区恢复到上次提交".to_string(), false),
            FileAction::RenameSymbol(plan) => (
                "重命名符号",
                format!(
                    "{}：{} 处，{} 个文件",
                    plan.name,
                    plan.count(),
                    plan.places.len()
                ),
                false,
            ),
            FileAction::RemoveWorktree(path) => ("删除工作树", self.remove_worktree_detail(path), true),
            FileAction::CleanupWorktree(path, branch) => (
                "合并完成",
                format!(
                    "改动已在主分支上。删除工作树 {} 和分支 {branch}？",
                    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
                ),
                true,
            ),
            FileAction::Rollback(path) => (
                "回滚",
                format!("将 {path} 恢复为上次提交的版本。未提交的修改将丢失，新文件将移到回收站。"),
                true,
            ),
        };
        let panel = v_flex()
            .id("file-prompt")
            .key_context("FilePrompt")
            .track_focus(&prompt.focus)
            .occlude()
            .w(px(460.))
            .p_4()
            .gap_3()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .text_size(UI_TEXT)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
            .child(
                div()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .child(detail),
            )
            .when(!delete, |panel| panel.child(Input::new(&prompt.input).small()))
            .when_some(prompt.error.clone(), |panel, error| {
                panel.child(div().text_size(UI_SMALL).text_color(theme.danger).child(error))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("file-prompt-cancel")
                            .small()
                            .label(match &prompt.action {
                                FileAction::CleanupWorktree(..) => "保留",
                                _ => "取消",
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.close_file_prompt(window, cx))),
                    )
                    .child(
                        Button::new("file-prompt-ok")
                            .small()
                            .when(delete, |button| button.danger())
                            .when(!delete, |button| button.primary())
                            .label(match &prompt.action {
                                FileAction::Rollback(_) => "回滚",
                                FileAction::Delete(_) | FileAction::RemoveWorktree(_) | FileAction::CleanupWorktree(..) => "删除",
                                _ => "确定",
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.confirm_file_prompt(window, cx))),
                    ),
            );
        Some(
            div()
                .id("file-prompt-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_file_prompt(window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    /// Where the sidebar's "new file" button puts the file: the selected
    /// folder, the folder of the selected file, or the project root.
    pub(super) fn new_file_dir(&self, cx: &App) -> PathBuf {
        match self.tree.read(cx).selected() {
            Some(path) if path.is_dir() => path.to_path_buf(),
            Some(path) => path.parent().map_or(self.root.clone(), Path::to_path_buf),
            None => self.root.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{below, package_for};

    #[test]
    fn names_stay_inside_the_folder() {
        let dir = Path::new("/p");
        assert_eq!(below(dir, "a.go"), Some(PathBuf::from("/p/a.go")));
        assert_eq!(below(dir, r"api\h.go"), Some(PathBuf::from("/p/api/h.go")));
        for bad in ["", "  ", "../x.go", "/abs.go", "a//b", "C:x", "./a"] {
            assert_eq!(below(dir, bad), None, "{bad}");
        }
    }

    #[test]
    fn a_new_go_file_takes_the_package_of_its_folder() {
        let base = std::env::temp_dir().join(format!("gust-package-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("store")).unwrap();
        std::fs::create_dir_all(base.join("My-Cache")).unwrap();
        std::fs::write(base.join("go.mod"), "module x\n").unwrap();
        std::fs::write(base.join("store/a.go"), "// doc\npackage storage\n").unwrap();
        assert_eq!(package_for(&base), "main");
        assert_eq!(package_for(&base.join("store")), "storage");
        assert_eq!(package_for(&base.join("My-Cache")), "mycache");
        let _ = std::fs::remove_dir_all(&base);
    }
}
