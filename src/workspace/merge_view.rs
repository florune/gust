//! Resolving merge conflicts from the commit panel, as GoLand does: a list
//! of the conflicted files, and for each a merge window with the incoming
//! side on the left and the current side on the right. Each conflict is
//! kept or dropped on each side (✓ / ✗); applying writes the file and marks
//! it resolved. When the list is empty, the merge can be committed.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::git::ChangeKind;
use crate::merge::{self, Row};
use crate::ui::{UI_SMALL, UI_TEXT, file_icon, icon};

actions!(gust, [CloseMergeDialog]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseMergeDialog, Some("MergeDialog"))]);
}

/// Height of a row of the merge window.
const ROW: f32 = 22.;

/// Whether a side of a conflict is kept, dropped, or not decided yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Decision {
    Pending,
    Keep,
    Drop,
}

/// The merge window of one file.
pub(super) struct MergeWindow {
    /// From the repository root.
    pub path: String,
    pub file: PathBuf,
    /// The file as it is, conflict markers and all.
    pub text: String,
    pub rows: Vec<Row>,
    /// For each conflict: the left (incoming) and right (current) side.
    pub decisions: Vec<(Decision, Decision)>,
    pub left_label: SharedString,
    pub right_label: SharedString,
    scroll: UniformListScrollHandle,
}

/// The list of conflicted files, open over the window.
pub(super) struct MergeDialog {
    pub merge: Option<MergeWindow>,
    focus: FocusHandle,
}

impl Workspace {
    /// Paths, from the repository root, that git reports as conflicted.
    pub(super) fn conflicted_paths(&self) -> Vec<String> {
        self.git
            .iter()
            .flat_map(|status| &status.changes)
            .filter(|change| change.kind == ChangeKind::Conflict)
            .map(|change| change.path.clone())
            .collect()
    }

    /// A conflicted file is staged as soon as it is saved without markers.
    pub(super) fn stage_if_resolved(&mut self, file: &std::path::Path, text: &str, cx: &mut Context<Self>) {
        let Some((root, path)) = self.repo_path(file) else {
            return;
        };
        if !self.conflicted_paths().contains(&path) || !merge::conflicts(text).is_empty() {
            return;
        }
        self.stage_resolved(root, path, cx);
    }

    fn stage_resolved(&mut self, root: PathBuf, path: String, cx: &mut Context<Self>) {
        let staging = cx
            .background_executor()
            .spawn(async move { crate::git::mark_resolved(&root, &path) });
        cx.spawn(async move |this, cx| {
            let result = staging.await;
            this.update(cx, |this, cx| {
                if let Err(reason) = result {
                    this.notice = Some(reason.into());
                }
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ dialog

    pub(super) fn open_merge_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_all(cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.merge_dialog = Some(MergeDialog { merge: None, focus });
        cx.notify();
    }

    pub(super) fn on_close_merge_dialog(&mut self, _: &CloseMergeDialog, window: &mut Window, cx: &mut Context<Self>) {
        match &mut self.merge_dialog {
            Some(dialog) if dialog.merge.is_some() => dialog.merge = None,
            _ => {
                self.merge_dialog = None;
                self.focus_editor(window, cx);
            }
        }
        cx.notify();
    }

    /// Take one whole side of a conflicted file, and mark it resolved.
    pub(super) fn take_side(&mut self, path: String, ours: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(file) = self.git.as_ref().map(|status| status.root.join(&path))
            && let Ok(text) = std::fs::read_to_string(&file)
        {
            self.snapshot_before_change(&file, &text.replace("\r\n", "\n"), "合并前");
        }
        self.run_git(window, cx, move |root| crate::git::take_side(&root, &path, ours), |this, window, cx| {
            this.files_changed(window, cx)
        });
    }

    /// Open the merge window of `path`, a conflicted file.
    pub(super) fn open_merge(&mut self, path: String, cx: &mut Context<Self>) {
        let Some(root) = self.git.as_ref().map(|status| status.root.clone()) else {
            return;
        };
        let file = root.join(&path);
        let Ok(raw) = std::fs::read_to_string(&file) else {
            self.notice = Some(format!("无法读取 {path}").into());
            cx.notify();
            return;
        };
        let text = raw.replace("\r\n", "\n");
        let found = merge::conflicts(&text);
        if found.is_empty() {
            self.notice = Some(format!("{path} 中没有冲突标记，可直接采用一侧").into());
            cx.notify();
            return;
        }
        let branch = self.git.as_ref().map(|status| status.branch.clone()).unwrap_or_default();
        let ours = &found[0].ours_label;
        let right_label = if ours.is_empty() || ours == "HEAD" {
            format!("本地 · {branch}")
        } else {
            format!("本地 · {ours}")
        };
        let left_label = match found[0].theirs_label.as_str() {
            "" => "传入".to_string(),
            label => format!("传入 · {label}"),
        };
        let merge = MergeWindow {
            path,
            file,
            rows: merge::rows(&text),
            decisions: vec![(Decision::Pending, Decision::Pending); found.len()],
            text,
            left_label: left_label.into(),
            right_label: right_label.into(),
            scroll: UniformListScrollHandle::new(),
        };
        // Open at the first conflict.
        if let Some(first) = merge.rows.iter().position(|row| matches!(row, Row::Conflict { .. })) {
            merge.scroll.scroll_to_item(first.saturating_sub(3), ScrollStrategy::Top);
        }
        if let Some(dialog) = &mut self.merge_dialog {
            dialog.merge = Some(merge);
        }
        cx.notify();
    }

    /// Decide a side of conflict `index`: `left` is the incoming side.
    pub(super) fn decide(&mut self, index: usize, left: bool, decision: Decision, cx: &mut Context<Self>) {
        let Some(merge) = self.merge_dialog.as_mut().and_then(|dialog| dialog.merge.as_mut()) else {
            return;
        };
        if let Some(pair) = merge.decisions.get_mut(index) {
            let side = if left { &mut pair.0 } else { &mut pair.1 };
            // Deciding the same again takes it back.
            *side = if *side == decision { Decision::Pending } else { decision };
        }
        cx.notify();
    }

    /// Decide every conflict the same way: keep one side and drop the other.
    fn decide_all(&mut self, keep_left: bool, cx: &mut Context<Self>) {
        let Some(merge) = self.merge_dialog.as_mut().and_then(|dialog| dialog.merge.as_mut()) else {
            return;
        };
        let (keep, drop) = (Decision::Keep, Decision::Drop);
        for pair in &mut merge.decisions {
            *pair = if keep_left { (keep, drop) } else { (drop, keep) };
        }
        cx.notify();
    }

    /// Scroll to the next conflict not decided yet, or the last one.
    fn step_conflict(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(merge) = self.merge_dialog.as_mut().and_then(|dialog| dialog.merge.as_mut()) else {
            return;
        };
        let firsts: Vec<usize> = merge
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| matches!(row, Row::Conflict { first: true, .. }))
            .map(|(ix, _)| ix)
            .collect();
        let pending: Vec<usize> = merge
            .decisions
            .iter()
            .enumerate()
            .filter(|(_, pair)| pair.0 == Decision::Pending || pair.1 == Decision::Pending)
            .map(|(ix, _)| ix)
            .collect();
        let target = if forward { pending.first() } else { pending.last() }.or(firsts.first());
        if let Some(&index) = target
            && let Some(&row) = firsts.get(index)
        {
            merge.scroll.scroll_to_item(row.saturating_sub(3), ScrollStrategy::Top);
        }
        cx.notify();
    }

    /// Write the file with every conflict settled, and mark it resolved.
    pub(super) fn apply_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(merge) = self.merge_dialog.as_mut().and_then(|dialog| dialog.merge.take()) else {
            return;
        };
        let choices: Vec<merge::Choice> = merge
            .decisions
            .iter()
            .map(|(left, right)| merge::choice(*left == Decision::Keep, *right == Decision::Keep))
            .collect();
        let resolved = merge::resolve_all(&merge.text, &choices);
        self.snapshot_before_change(&merge.file, &merge.text, "合并前");
        match self.docs.iter().position(|doc| doc.path == merge.file) {
            Some(ix) => {
                let editor = self.docs[ix].editor.clone();
                let before = editor.read(cx).text().to_string();
                self.apply_as_one_edit(&editor, &before, &resolved, None, window, cx);
                self.write_doc(ix, cx);
            }
            None => {
                let crlf = std::fs::read_to_string(&merge.file).is_ok_and(|raw| raw.contains("\r\n"));
                let text = if crlf { resolved.replace('\n', "\r\n") } else { resolved };
                if let Err(err) = std::fs::write(&merge.file, text) {
                    self.notice = Some(format!("无法写入 {}：{err}", merge.path).into());
                    cx.notify();
                    return;
                }
            }
        }
        if let Some(root) = self.git.as_ref().map(|status| status.root.clone()) {
            self.stage_resolved(root, merge.path, cx);
        }
        cx.notify();
    }

    // ------------------------------------------------------------ rendering

    pub(super) fn render_merge_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let dialog = self.merge_dialog.as_ref()?;
        let panel = match &dialog.merge {
            Some(merge) => self.render_merge_window(merge, cx),
            None => self.render_conflict_list(cx),
        };
        Some(
            div()
                .id("merge-layer")
                .key_context("MergeDialog")
                .track_focus(&dialog.focus)
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_center()
                .bg(black().opacity(0.35))
                .occlude()
                .child(panel)
                .into_any_element(),
        )
    }

    fn render_conflict_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let paths = self.conflicted_paths();
        let done = paths.is_empty();
        let rows = paths.iter().enumerate().map(|(ix, path)| {
            let (folder, name) = match path.rsplit_once('/') {
                Some((folder, name)) => (folder.to_string(), name.to_string()),
                None => (String::new(), path.clone()),
            };
            let (ours, theirs, open) = (path.clone(), path.clone(), path.clone());
            h_flex()
                .id(("conflict-file", ix))
                .h(px(34.))
                .px_2()
                .gap_2()
                .rounded(px(5.))
                .hover(|row| row.bg(theme.list_hover))
                .child(file_icon(&name, cx).small())
                .child(div().flex_shrink_0().text_size(UI_TEXT).child(name))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(UI_SMALL)
                        .text_color(theme.muted_foreground)
                        .child(folder),
                )
                .child(
                    Button::new(("take-ours", ix))
                        .xsmall()
                        .label("采用本地")
                        .on_click(cx.listener(move |this, _, window, cx| this.take_side(ours.clone(), true, window, cx))),
                )
                .child(
                    Button::new(("take-theirs", ix))
                        .xsmall()
                        .label("采用传入")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.take_side(theirs.clone(), false, window, cx)
                        })),
                )
                .child(
                    Button::new(("merge-file", ix))
                        .xsmall()
                        .primary()
                        .label("合并…")
                        .on_click(cx.listener(move |this, _, _, cx| this.open_merge(open.clone(), cx))),
                )
        });
        let close = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.merge_dialog = None;
            this.focus_editor(window, cx);
            cx.notify();
        };
        v_flex()
            .id("merge-dialog")
            .w(px(720.))
            .max_h(relative(0.85))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(icon("git-merge").small())
                    .child(div().flex_1().font_weight(FontWeight::SEMIBOLD).text_size(UI_TEXT).child("合并冲突"))
                    .child(
                        div()
                            .text_size(UI_SMALL)
                            .text_color(theme.muted_foreground)
                            .child(if done {
                                "所有冲突已解决".to_string()
                            } else {
                                format!("{} 个文件有冲突", paths.len())
                            }),
                    ),
            )
            .child(
                v_flex()
                    .id("conflict-files")
                    .flex_1()
                    .min_h(px(120.))
                    .overflow_y_scroll()
                    .p_2()
                    .children(rows)
                    .when(done, |list| {
                        list.child(
                            div()
                                .p_4()
                                .text_size(UI_TEXT)
                                .text_color(theme.muted_foreground)
                                .child("没有待解决的冲突文件"),
                        )
                    }),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .justify_end()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("merge-dialog-close")
                            .small()
                            .label("关闭")
                            .on_click(cx.listener(move |this, _, window, cx| close(this, window, cx))),
                    )
                    .child(
                        Button::new("merge-done")
                            .small()
                            .primary()
                            .label("解决完成")
                            .disabled(!done)
                            .on_click(cx.listener(move |this, _, window, cx| close(this, window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_merge_window(&self, merge: &MergeWindow, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let total = merge.decisions.len();
        let decided = merge
            .decisions
            .iter()
            .filter(|(left, right)| *left != Decision::Pending && *right != Decision::Pending)
            .count();
        let label = |text: SharedString| {
            div()
                .flex_1()
                .min_w_0()
                .px_3()
                .py_1()
                .truncate()
                .text_size(UI_SMALL)
                .font_weight(FontWeight::SEMIBOLD)
                .child(text)
        };
        let count = merge.rows.len();
        let list = uniform_list(
            "merge-rows",
            count,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.render_merge_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&merge.scroll)
        .flex_1()
        .min_h_0();

        v_flex()
            .id("merge-window")
            .size_full()
            .m(px(24.))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(icon("git-merge").small())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(UI_TEXT)
                            .child(format!("合并 {} · 冲突之外双方的改动均已保留", merge.path)),
                    )
                    .child(
                        div()
                            .text_size(UI_SMALL)
                            .text_color(theme.muted_foreground)
                            .child(format!("已决定 {decided} / {total} 处冲突")),
                    )
                    .child(
                        Self::header_button("merge-previous", "arrow-up", "上一处冲突", cx)
                            .on_click(cx.listener(|this, _, _, cx| this.step_conflict(false, cx))),
                    )
                    .child(
                        Self::header_button("merge-next", "arrow-down", "下一处未决定的冲突", cx)
                            .on_click(cx.listener(|this, _, _, cx| this.step_conflict(true, cx))),
                    ),
            )
            .child(
                h_flex()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.sidebar)
                    .child(label(merge.left_label.clone()))
                    .child(div().w(px(1.)).h_full().bg(theme.border))
                    .child(label(merge.right_label.clone())),
            )
            .child(list)
            .child(
                h_flex()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("merge-all-left")
                            .small()
                            .label("全部采用左侧")
                            .on_click(cx.listener(|this, _, _, cx| this.decide_all(true, cx))),
                    )
                    .child(
                        Button::new("merge-all-right")
                            .small()
                            .label("全部采用右侧")
                            .on_click(cx.listener(|this, _, _, cx| this.decide_all(false, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("merge-cancel")
                            .small()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(dialog) = &mut this.merge_dialog {
                                    dialog.merge = None;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("merge-apply")
                            .small()
                            .primary()
                            .label("应用")
                            .disabled(decided < total)
                            .on_click(cx.listener(|this, _, window, cx| this.apply_merge(window, cx))),
                    ),
            )
            .into_any_element()
    }

    fn render_merge_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(merge) = self.merge_dialog.as_ref().and_then(|dialog| dialog.merge.as_ref()) else {
            return div().into_any_element();
        };
        let colors = crate::theme::palette(cx);
        let (mono, muted, border) = {
            let theme = cx.theme();
            (theme.mono_font_family.clone(), theme.muted_foreground, theme.border)
        };
        let cell = |number: Option<usize>, text: &str| {
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .overflow_hidden()
                .child(
                    div()
                        .w(px(48.))
                        .flex_shrink_0()
                        .pr_3()
                        .text_right()
                        .text_color(muted)
                        .child(number.map(|n| n.to_string()).unwrap_or_default()),
                )
                .child(div().whitespace_nowrap().child(text.replace('\t', "    ")))
        };
        let row = h_flex().h(px(ROW)).w_full().font_family(mono).text_size(UI_TEXT);
        match &merge.rows[ix] {
            Row::Same { left, right, text } => row
                .child(cell(Some(*left), text))
                .child(div().w(px(1.)).h_full().bg(border))
                .child(cell(Some(*right), text))
                .into_any_element(),
            Row::Conflict { index, left, right, first } => {
                let index = *index;
                let first = *first;
                let (left_decision, right_decision) = merge.decisions[index];
                let tint = |decision: Decision| match decision {
                    Decision::Pending => colors.red.opacity(0.18),
                    Decision::Keep => colors.green.opacity(0.18),
                    Decision::Drop => muted.opacity(0.08),
                };
                let side = |line: &Option<merge::Line>, decision: Decision, is_left: bool| {
                    let (number, text) = match line {
                        Some(line) => (Some(line.number), line.text.clone()),
                        None => (None, String::new()),
                    };
                    let mark = |id: &'static str, glyph: &'static str, tip: &'static str, color: Hsla, choice: Decision| {
                        let chosen = decision == choice;
                        div()
                            .id((id, index))
                            .size(px(18.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .text_color(if chosen { gpui_kit::white() } else { color })
                            .when(chosen, |mark| mark.bg(color))
                            .when(!chosen, |mark| mark.hover(|mark| mark.bg(color.opacity(0.2))))
                            .child(icon(glyph).xsmall())
                            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                            .on_click(cx.listener(move |this, _, _, cx| this.decide(index, is_left, choice, cx)))
                    };
                    let marks = first.then(|| {
                        h_flex()
                            .absolute()
                            .right(px(6.))
                            .top(px(1.))
                            .gap_1()
                            .child(mark(
                                if is_left { "drop-left" } else { "drop-right" },
                                "x",
                                "不要这一侧",
                                colors.red,
                                Decision::Drop,
                            ))
                            .child(mark(
                                if is_left { "keep-left" } else { "keep-right" },
                                "check",
                                "保留这一侧",
                                colors.green,
                                Decision::Keep,
                            ))
                    });
                    cell(number, &text)
                        .relative()
                        .bg(tint(decision))
                        .when(decision == Decision::Drop, |cell| cell.line_through().text_color(muted))
                        .when(first, |cell| cell.border_t_1().border_color(colors.red.opacity(0.6)))
                        .children(marks)
                };
                let left_cell = side(left, left_decision, true);
                let right_cell = side(right, right_decision, false);
                row.child(left_cell)
                    .child(div().w(px(1.)).h_full().bg(border))
                    .child(right_cell)
                    .into_any_element()
            }
        }
    }
}
