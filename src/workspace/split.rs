//! A second editor beside the first. The right pane has its own editor of
//! the file it shows, kept in step with the file's main editor, so the same
//! file can be read in two places at once. Files opened while the right pane
//! has the keyboard open there.

use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Editor, EditorState, InputEvent, TabSize};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use super::Workspace;
use crate::ui::{UI_TEXT, file_icon};

actions!(gust, [SplitRight, CloseSplit]);

pub(super) struct Split {
    pub path: PathBuf,
    pub editor: Entity<EditorState>,
    /// The right pane, not the left, was used last.
    pub focused: bool,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub(super) fn on_split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.active.and_then(|ix| self.docs.get(ix)).map(|doc| doc.path.clone()) {
            self.split_right(&path, window, cx);
        }
    }

    pub(super) fn on_close_split(&mut self, _: &CloseSplit, window: &mut Window, cx: &mut Context<Self>) {
        self.close_split(window, cx);
    }

    /// Show the open file `path` in the right pane, and give it the keyboard.
    pub(super) fn split_right(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.iter().find(|doc| doc.path == path) else {
            return;
        };
        if let Some(split) = &mut self.split
            && split.path == path
        {
            split.focused = true;
            split.editor.update(cx, |editor, cx| editor.focus(window, cx));
            cx.notify();
            return;
        }
        let main = doc.editor.clone();
        let language = doc.language;
        let text = main.read(cx).text().to_string();
        let position = main.read(cx).cursor_position();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language)
                .tab_size(TabSize {
                    tab_size: 4,
                    hard_tabs: matches!(language, "go" | "make"),
                })
                .soft_wrap(false)
                .default_value(text)
        });
        if language == "go" {
            self.attach_go(&editor, path, cx);
        }
        // Either editor's edits are copied into the other.
        let follow = |from: &Entity<EditorState>, to: Entity<EditorState>, window: &mut Window, cx: &mut Context<Self>| {
            cx.subscribe_in(from, window, move |this, from, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::Change => {
                        let after = from.read(cx).text().to_string();
                        let before = to.read(cx).text().to_string();
                        super::replace_text(&to, &before, &after, window, cx);
                    }
                    InputEvent::Focus => {
                        let right = this.split.as_ref().is_some_and(|split| &split.editor == from);
                        if let Some(split) = &mut this.split {
                            split.focused = right;
                            cx.notify();
                        }
                    }
                    _ => {}
                }
            })
        };
        let subscriptions = vec![
            follow(&editor, main.clone(), window, cx),
            follow(&main, editor.clone(), window, cx),
        ];
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(position, window, cx);
            editor.focus(window, cx);
        });
        self.split = Some(Split {
            path: path.to_path_buf(),
            editor,
            focused: true,
            _subscriptions: subscriptions,
        });
        self.diff_active = false;
        cx.notify();
    }

    pub(super) fn close_split(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.split.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    /// Whether a file opened now goes to the right pane.
    pub(super) fn split_focused(&self) -> bool {
        self.split.as_ref().is_some_and(|split| split.focused)
    }

    /// A tab was clicked: show its file in the pane that was used last.
    pub(super) fn select_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.docs.get(ix) {
            Some(doc) if self.split_focused() && !self.diff_active => {
                let path = doc.path.clone();
                self.split_right(&path, window, cx);
            }
            _ => self.activate(ix, window, cx),
        }
    }

    /// The right pane: the file's name, a button that closes the pane, and its editor.
    pub(super) fn render_split(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let split = self.split.as_ref()?;
        let doc = self.docs.iter().find(|doc| doc.path == split.path)?;
        let theme = cx.theme();
        let header = h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .pl_3()
            .pr_1()
            .gap_1p5()
            .text_size(UI_TEXT)
            .text_color(if split.focused {
                theme.foreground
            } else {
                theme.muted_foreground
            })
            .bg(theme.sidebar)
            .border_b_1()
            .border_color(theme.border)
            .child(file_icon(&doc.title, cx).small())
            .child(div().flex_1().min_w_0().truncate().child(doc.title.clone()))
            .child(
                Self::header_button("split-close", "x", "关闭拆分", cx)
                    .on_click(cx.listener(|this, _, window, cx| this.close_split(window, cx))),
            );
        Some(
            v_flex()
                .id("split-pane")
                .flex_1()
                .min_w_0()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .child(header)
                .child(
                    div().flex_1().min_h_0().child(
                        Editor::new(&split.editor)
                            .appearance(false)
                            .bordered(false)
                            .size_full(),
                    ),
                )
                .into_any_element(),
        )
    }

    /// The button at the end of the tab bar that splits the active file to the right.
    pub(super) fn render_split_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.active.is_none() || self.diff_active {
            return None;
        }
        Some(
            div()
                .px_1()
                .child(
                    Self::header_button("split-right", "columns-2", "向右拆分", cx).on_click(
                        cx.listener(|this, _, window, cx| this.on_split_right(&SplitRight, window, cx)),
                    ),
                )
                .into_any_element(),
        )
    }
}
