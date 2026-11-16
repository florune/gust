//! Rollback to the last commit: a block of changed lines (from its gutter
//! mark or Ctrl+Alt+Z), or a whole file (from the commit panel).

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::compare::{Chunk, ChunkKind, compare, take_lines};
use crate::ui::{UI_SMALL, UI_TEXT};

actions!(gust, [RollbackLines]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "ctrl-alt-z",
        RollbackLines,
        Some("Input && mode == editor"),
    )]);
}

/// Width from the gutter's left edge where the change marks are drawn.
const MARK_ZONE: f32 = 14.;

/// The committed lines of one change, shown beside its mark.
pub(super) struct Hunk {
    pub path: PathBuf,
    pub chunk: Chunk,
    pub old: String,
    pub at: Point<Pixels>,
}

impl Workspace {
    /// The change of the open file at `ix` that line `row` is part of.
    fn chunk_at(&self, ix: usize, row: usize, cx: &App) -> Option<Chunk> {
        let doc = self.docs.get(ix)?;
        let head = doc.head.as_ref()?;
        let text = doc.editor.read(cx).text().to_string();
        compare(head, &text).chunks.into_iter().find(|chunk| {
            chunk.new.contains(&row)
                // Removed lines are marked on the boundary where they were.
                || (chunk.new.is_empty() && (chunk.new.start == row || chunk.new.start == row + 1))
        })
    }

    /// Put the committed lines of `chunk` back into the open file at `ix`.
    fn roll_back(&mut self, ix: usize, chunk: &Chunk, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let Some(head) = doc.head.clone() else {
            return;
        };
        let editor = doc.editor.clone();
        let text = editor.read(cx).text().to_string();
        let restored = take_lines(&head, &text, chunk);
        self.apply_as_one_edit(&editor, &text, &restored, None, window, cx);
        self.hunk = None;
        cx.notify();
    }

    pub(super) fn on_rollback_lines(&mut self, _: &RollbackLines, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else {
            return;
        };
        let row = self.docs[ix].editor.read(cx).cursor_position().line as usize;
        if let Some(chunk) = self.chunk_at(ix, row, cx) {
            self.roll_back(ix, &chunk, window, cx);
        }
    }

    /// A click on a change mark beside the line numbers shows what was there.
    pub(super) fn gutter_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.modifiers;
        if event.button != MouseButton::Left
            || modifiers.control
            || modifiers.alt
            || modifiers.shift
            || self.diff_active
        {
            return;
        }
        let Some(ix) = self.active else {
            return;
        };
        let doc = &self.docs[ix];
        if doc.head.is_none() {
            return;
        }
        let (input, text_bounds, height) = {
            let editor = doc.editor.read(cx);
            (editor.input_bounds(), editor.text_bounds(), editor.line_height())
        };
        let (Some(text_bounds), Some(height)) = (text_bounds, height) else {
            return;
        };
        let position = event.position;
        let in_zone = position.x >= input.left()
            && position.x <= input.left() + px(MARK_ZONE)
            && input.contains(&position);
        if !in_zone || height <= px(0.) {
            return;
        }
        let row = ((position.y - text_bounds.top()) / height).floor().max(0.) as usize;
        let Some(chunk) = self.chunk_at(ix, row, cx) else {
            return;
        };
        let head = doc.head.clone().unwrap_or_default();
        let old: String = head
            .split_inclusive('\n')
            .skip(chunk.old.start)
            .take(chunk.old.len())
            .collect();
        self.hunk = Some(Hunk {
            path: doc.path.clone(),
            chunk,
            old,
            at: point(position.x + px(12.), position.y),
        });
        cx.stop_propagation();
        cx.notify();
    }

    pub(super) fn render_hunk(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let hunk = self.hunk.as_ref()?;
        let theme = cx.theme();
        let colors = crate::theme::palette(cx);
        let start = hunk.chunk.new.start + 1;
        let title = match hunk.chunk.kind {
            ChunkKind::Added => format!("第 {start} 行：新增 {} 行", hunk.chunk.new.len()),
            ChunkKind::Removed => format!("第 {start} 行：删除 {} 行", hunk.chunk.old.len()),
            ChunkKind::Modified => format!("第 {start} 行：修改 {} 行", hunk.chunk.new.len()),
        };
        let path = hunk.path.clone();
        let chunk = hunk.chunk.clone();
        let panel = v_flex()
            .id("hunk")
            .absolute()
            .left(hunk.at.x)
            .top(hunk.at.y)
            .w(px(560.))
            .max_w_full()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().child(title))
                    .child(
                        Button::new("hunk-compare")
                            .xsmall()
                            .label("对比")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let relative = this
                                    .repo_path(&path)
                                    .map(|(_, relative)| relative);
                                this.hunk = None;
                                this.open_diff(relative, window, cx);
                            })),
                    )
                    .child(
                        Button::new("hunk-rollback")
                            .xsmall()
                            .primary()
                            .label("回滚")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(ix) = this.active {
                                    this.roll_back(ix, &chunk, window, cx);
                                }
                            })),
                    ),
            )
            .when(!hunk.old.is_empty(), |panel| {
                panel.child(
                    div()
                        .id("hunk-old")
                        .max_h(px(260.))
                        .overflow_y_scroll()
                        .px_3()
                        .py_2()
                        .bg(colors.red.opacity(0.08))
                        .font_family(theme.mono_font_family.clone())
                        .text_size(UI_TEXT)
                        .whitespace_nowrap()
                        .children(hunk.old.lines().map(|line| div().child(line.replace('\t', "    ")))),
                )
            });
        Some(
            div()
                .id("hunk-layer")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.hunk = None;
                        cx.notify();
                    }),
                )
                .child(panel)
                .into_any_element(),
        )
    }

    /// Ask before taking the file at `path`, from the repository root, back
    /// to the last commit.
    pub(super) fn ask_rollback_file(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_prompt(super::file_ops::FileAction::Rollback(path), window, cx);
    }

    /// Take the file back to the last commit; what was open from it follows.
    pub(super) fn rollback_file(&mut self, path: &str, window: &mut Window, cx: &mut Context<Self>) -> Result<(), String> {
        let status = self.git.as_ref().ok_or("当前文件夹不是 Git 仓库")?;
        let change = status
            .changes
            .iter()
            .find(|change| change.path == path)
            .cloned()
            .ok_or("文件没有改动")?;
        let root = status.root.clone();
        let file = root.join(&change.path);
        if let Some(text) = self.docs.iter().find(|doc| doc.path == file).map(|doc| doc.editor.read(cx).text().to_string()).or_else(|| std::fs::read_to_string(&file).ok()) {
            self.snapshot_before_change(&file, &text, "回滚前");
        }
        crate::git::rollback(&root, &change)?;
        // Open files take the restored text and drop their unsaved edits.
        for file in [Some(change.path.clone()), change.old_path.clone()].into_iter().flatten() {
            let absolute = root.join(&file);
            if let Some(ix) = self.docs.iter().position(|doc| doc.path == absolute) {
                match std::fs::read_to_string(&absolute) {
                    Ok(text) => {
                        let text = text.replace("\r\n", "\n");
                        let doc = &mut self.docs[ix];
                        doc.conflict = None;
                        doc.saved = text.clone();
                        let editor = doc.editor.clone();
                        let before = editor.read(cx).text().to_string();
                        super::replace_text(&editor, &before, &text, window, cx);
                        self.refresh_dirty(&editor, cx);
                    }
                    Err(_) => {
                        self.docs[ix].dirty = false;
                        self.close(ix, window, cx);
                    }
                }
            }
        }
        self.refresh_git(cx);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn roll_back_for_test(&mut self, chunk: &Chunk, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.active {
            self.roll_back(ix, chunk, window, cx);
        }
    }
}
