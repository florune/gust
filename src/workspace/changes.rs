//! Git in the workspace: the changes panel, the diff tab, the change marks
//! beside line numbers, and the branch in the status bar.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{GutterDecoration, Textarea};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::git::{self, ChangeKind};
use crate::ui::icon;

/// More changed files than this are not listed one by one.
const MAX_LISTED: usize = 400;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SidebarTab {
    Files,
    Changes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LineChange {
    Added,
    Modified,
    /// Lines were removed above this line.
    Deleted,
}

/// How the lines of `current` differ from `head`: zero-based line ranges.
/// A deletion is an empty range at the line the removed text sat above.
pub(super) fn changed_lines(head: &str, current: &str) -> Vec<(Range<usize>, LineChange)> {
    similar::TextDiff::from_lines(head, current)
        .ops()
        .iter()
        .filter_map(|op| {
            let range = op.new_range();
            match op {
                similar::DiffOp::Equal { .. } => None,
                similar::DiffOp::Insert { .. } => Some((range, LineChange::Added)),
                similar::DiffOp::Delete { .. } => Some((range, LineChange::Deleted)),
                similar::DiffOp::Replace { .. } => Some((range, LineChange::Modified)),
            }
        })
        .collect()
}

fn kind_color(kind: ChangeKind, cx: &App) -> Hsla {
    let theme = cx.theme();
    match kind {
        ChangeKind::Modified | ChangeKind::Renamed => theme.blue,
        ChangeKind::Added | ChangeKind::Untracked => theme.green,
        ChangeKind::Deleted => theme.muted_foreground,
        ChangeKind::Conflict => theme.red,
    }
}

impl Workspace {
    /// Ask git for the repository's state in the background.
    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        let loading = cx.background_executor().spawn(async move {
            let status = git::status(&root);
            let branches = status
                .as_ref()
                .map(|status| git::branches(&status.root))
                .unwrap_or_default();
            (status, branches)
        });
        // Replacing the task drops an older refresh that has not finished.
        self.git_refresh = cx.spawn(async move |this, cx| {
            let (status, branches) = loading.await;
            this.update(cx, |this, cx| {
                this.branches = branches;
                if this.git != status {
                    this.apply_git_status(status, cx);
                }
            })
            .ok();
        });
    }

    fn apply_git_status(&mut self, status: Option<git::Status>, cx: &mut Context<Self>) {
        let mut files = HashMap::new();
        let mut folders = HashSet::new();
        if let Some(status) = &status {
            for change in &status.changes {
                let path = status.root.join(&change.path);
                for folder in path.ancestors().skip(1) {
                    if !folders.insert(folder.to_path_buf()) || folder == status.root {
                        break;
                    }
                }
                files.insert(path, kind_color(change.kind, cx));
            }
            self.excluded
                .retain(|path| status.changes.iter().any(|change| &change.path == path));
        } else {
            self.excluded.clear();
        }
        let folder_color = cx.theme().blue;
        self.tree.update(cx, |tree, cx| {
            tree.set_changes(files, folders, folder_color, cx)
        });
        self.git = status;

        for ix in 0..self.docs.len() {
            self.load_head(ix, cx);
        }
        // A commit's changes never change; the working tree's do.
        self.refresh_working_diff(cx);
        if self.log_visible() {
            self.reload_log(cx);
        }
        cx.notify();
    }

    /// Path of a file from the repository root, as git names it.
    fn repo_path(&self, path: &std::path::Path) -> Option<(PathBuf, String)> {
        let root = &self.git.as_ref()?.root;
        let relative = path.strip_prefix(root).ok()?;
        Some((root.clone(), relative.to_string_lossy().replace('\\', "/")))
    }

    /// Fetch the committed text of an open file, which the change marks compare against.
    pub(super) fn load_head(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let editor = doc.editor.clone();
        let Some((root, path)) = self.repo_path(&doc.path) else {
            self.docs[ix].head = None;
            self.refresh_gutter(ix, cx);
            return;
        };
        let loading = cx
            .background_executor()
            .spawn(async move { git::head_text(&root, &path) });
        cx.spawn(async move |this, cx| {
            let head = loading.await.map(|head| head.replace("\r\n", "\n"));
            this.update(cx, |this, cx| {
                if let Some(ix) = this.docs.iter().position(|doc| doc.editor == editor) {
                    this.docs[ix].head = head;
                    this.refresh_gutter(ix, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Redraw the marks beside the line numbers of an open file.
    pub(super) fn refresh_gutter(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let (added, modified, deleted) = {
            let theme = cx.theme();
            (theme.green, theme.blue, theme.red)
        };
        let decorations = match &doc.head {
            None => Vec::new(),
            Some(head) => {
                let text = doc.editor.read(cx).text().to_string();
                let mut starts = vec![0];
                starts.extend(text.match_indices('\n').map(|(ix, _)| ix + 1));
                let offset = |line: usize| starts.get(line).copied().unwrap_or(text.len());
                changed_lines(head, &text)
                    .into_iter()
                    .map(|(lines, change)| match change {
                        LineChange::Deleted => {
                            let at = offset(lines.start);
                            GutterDecoration::new(at..at).with_color(deleted)
                        }
                        LineChange::Added | LineChange::Modified => {
                            let color = if change == LineChange::Added {
                                added
                            } else {
                                modified
                            };
                            // Up to, not including, the newline of the last line.
                            let end = offset(lines.end).saturating_sub(1).max(offset(lines.start));
                            GutterDecoration::new(offset(lines.start)..end.max(offset(lines.start) + 1).min(text.len()))
                                .with_color(color)
                        }
                    })
                    .collect()
            }
        };
        let editor = doc.editor.clone();
        match &doc.gutter {
            Some(collection) => collection.set(decorations, cx),
            None if decorations.is_empty() => {}
            None => {
                let collection = editor.update(cx, |editor, cx| {
                    editor.create_gutter_decorations_collection(decorations, cx)
                });
                self.docs[ix].gutter = Some(collection);
            }
        }
    }

    /// Run a git command off the main thread, then show its outcome and refresh.
    fn run_git(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        command: impl FnOnce(PathBuf) -> Result<(), String> + Send + 'static,
        done: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let Some(status) = &self.git else {
            return;
        };
        if self.git_busy {
            return;
        }
        self.git_busy = true;
        let root = status.root.clone();
        let running = cx.background_executor().spawn(async move { command(root) });
        cx.spawn_in(window, async move |this, cx| {
            let result = running.await;
            this.update_in(cx, |this, window, cx| {
                this.git_busy = false;
                match result {
                    Ok(()) => {
                        this.notice = None;
                        done(this, window, cx);
                    }
                    Err(reason) => this.notice = Some(reason.into()),
                }
                this.refresh_git(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // What is committed is what is on disk, so unsaved edits go there first.
        self.save_all(cx);
        let Some(status) = &self.git else {
            return;
        };
        let paths: Vec<String> = status
            .changes
            .iter()
            .filter(|change| !self.excluded.contains(&change.path))
            .map(|change| change.path.clone())
            .collect();
        let message = self.commit_message.read(cx).value().to_string();
        self.run_git(
            window,
            cx,
            move |root| git::commit(&root, &message, &paths),
            |this, window, cx| {
                this.commit_message
                    .update(cx, |input, cx| input.set_value("", window, cx));
            },
        );
    }

    fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_upstream = self.git.as_ref().is_some_and(|status| status.has_upstream);
        self.run_git(
            window,
            cx,
            move |root| git::push(&root, has_upstream),
            |_, _, _| {},
        );
    }

    fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_git(window, cx, |root| git::pull(&root), |_, _, _| {});
    }

    pub(super) fn switch_branch(&mut self, branch: String, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all(cx);
        self.run_git(
            window,
            cx,
            move |root| git::switch(&root, &branch),
            |_, _, _| {},
        );
    }

    /// The branch in the status bar; clicking it lists the branches to switch to.
    pub(super) fn render_branch(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let status = self.git.as_ref()?;
        let mut label = status.branch.clone();
        if status.ahead > 0 {
            label.push_str(&format!(" ↑{}", status.ahead));
        }
        if status.behind > 0 {
            label.push_str(&format!(" ↓{}", status.behind));
        }
        let current = status.branch.clone();
        let branches = self.branches.clone();
        let workspace = cx.entity().downgrade();
        Some(
            Button::new("branch")
                .ghost()
                .xsmall()
                .icon(icon("git-branch"))
                .label(label)
                .dropdown_menu(move |mut menu, _, _| {
                    for branch in &branches {
                        let workspace = workspace.clone();
                        let name = branch.clone();
                        menu = menu.item(
                            PopupMenuItem::new(branch.clone())
                                .checked(*branch == current)
                                .on_click(move |_, window, cx| {
                                    workspace
                                        .update(cx, |workspace, cx| {
                                            workspace.switch_branch(name.clone(), window, cx)
                                        })
                                        .ok();
                                }),
                        );
                    }
                    menu
                }),
        )
    }

    pub(super) fn render_changes(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(status) = &self.git else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("这个文件夹不是 Git 仓库")
                .into_any_element();
        };
        let selected = status
            .changes
            .iter()
            .filter(|change| !self.excluded.contains(&change.path))
            .count();
        let shown_diff = self
            .diff
            .as_ref()
            .filter(|_| self.diff_active)
            .and_then(|tab| tab.working_path());

        let rows = status
            .changes
            .iter()
            .take(MAX_LISTED)
            .enumerate()
            .map(|(ix, change)| {
                let (folder, name) = match change.path.rsplit_once('/') {
                    Some((folder, name)) => (folder.to_string(), name.to_string()),
                    None => (String::new(), change.path.clone()),
                };
                let path = change.path.clone();
                let toggled = change.path.clone();
                let active = shown_diff.as_deref() == Some(change.path.as_str());
                h_flex()
                    .id(("change", ix))
                    .h(px(26.))
                    .w_full()
                    .px_2()
                    .gap_1p5()
                    .text_sm()
                    .when(active, |row| row.bg(theme.list_active))
                    .hover(|row| row.bg(theme.list_hover))
                    .child(
                        Checkbox::new(("change-check", ix))
                            .checked(!self.excluded.contains(&change.path))
                            .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                if *checked {
                                    this.excluded.remove(&toggled);
                                } else {
                                    this.excluded.insert(toggled.clone());
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .when(change.kind == ChangeKind::Deleted, |name| {
                                name.line_through()
                            })
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(folder),
                    )
                    .when(change.added > 0, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(theme.green)
                                .child(format!("+{}", change.added)),
                        )
                    })
                    .when(change.removed > 0, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(theme.red)
                                .child(format!("−{}", change.removed)),
                        )
                    })
                    .child(
                        div()
                            .w_3()
                            .text_xs()
                            .text_color(kind_color(change.kind, cx))
                            .child(change.kind.letter()),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_diff(Some(path.clone()), window, cx)
                    }))
            });

        v_flex()
            .size_full()
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .child(Textarea::new(&self.commit_message).small()),
            )
            .child(
                h_flex()
                    .px_2()
                    .gap_1()
                    .child(
                        Button::new("commit")
                            .primary()
                            .small()
                            .label(format!("提交 {selected} 个文件"))
                            .loading(self.git_busy)
                            .disabled(selected == 0 || self.git_busy)
                            .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("pull")
                            .ghost()
                            .small()
                            .icon(icon("arrow-down"))
                            .tooltip("拉取")
                            .disabled(self.git_busy)
                            .on_click(cx.listener(|this, _, window, cx| this.pull(window, cx))),
                    )
                    .child(
                        Button::new("push")
                            .ghost()
                            .small()
                            .icon(icon("arrow-up"))
                            .tooltip("推送")
                            .disabled(self.git_busy)
                            .on_click(cx.listener(|this, _, window, cx| this.push(window, cx))),
                    ),
            )
            .child(
                h_flex()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!("{} 个改动", status.changes.len()))
                    .child(div().flex_1())
                    .when(!status.changes.is_empty(), |row| {
                        row.child(
                            div()
                                .id("all-changes")
                                .hover(|this| this.text_color(theme.foreground))
                                .child("查看全部")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_diff(None, window, cx)
                                })),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("changes")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(status.changes.len() > MAX_LISTED, |list| {
                        list.child(
                            div()
                                .px_3()
                                .py_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!(
                                    "还有 {} 个文件未列出",
                                    status.changes.len() - MAX_LISTED
                                )),
                        )
                    }),
            )
            .into_any_element()
    }

}

#[cfg(test)]
mod tests {
    use super::{LineChange, changed_lines};

    #[test]
    fn changed_lines_tell_additions_edits_and_deletions_apart() {
        let head = "a\nb\nc\nd\ne\n";
        let current = "a\nB\nc\ne\nf\ng\n";
        assert_eq!(
            changed_lines(head, current),
            [
                (1..2, LineChange::Modified),
                (3..3, LineChange::Deleted),
                (4..6, LineChange::Added),
            ]
        );
        assert!(changed_lines(head, head).is_empty());
    }
}
