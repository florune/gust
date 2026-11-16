//! Quick documentation (Ctrl+Q): the declaration and comment of the name at
//! the caret, in a popup beside it.

use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::goindex;
use crate::ui::{UI_SMALL, UI_TEXT};

actions!(gust, [QuickDocumentation]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "ctrl-q",
        QuickDocumentation,
        Some("Input && mode == editor"),
    )]);
}

pub(super) struct QuickDoc {
    pub doc: goindex::Documentation,
    at: Point<Pixels>,
}

impl Workspace {
    pub(super) fn on_quick_documentation(
        &mut self,
        _: &QuickDocumentation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let (source, offset, caret) = {
            let state = editor.read(cx);
            let offset = state.cursor();
            (state.text().to_string(), offset, state.range_to_bounds(&(offset..offset)))
        };
        let open: Vec<(std::path::PathBuf, String)> = self
            .docs
            .iter()
            .map(|doc| (doc.path.clone(), doc.editor.read(cx).text().to_string()))
            .collect();
        let read = |file: &std::path::Path| {
            open.iter()
                .find(|(path, _)| path == file)
                .map(|(_, text)| text.clone())
                .or_else(|| std::fs::read_to_string(file).ok())
        };
        let Some(doc) = goindex::documentation(&self.index, &path, &source, offset, &read) else {
            return;
        };
        let at = caret.map_or(point(px(200.), px(120.)), |bounds| {
            point(bounds.left(), bounds.bottom() + px(4.))
        });
        self.quick_doc = Some(QuickDoc { doc, at });
        cx.notify();
    }

    /// Any key or click elsewhere closes the popup.
    pub(super) fn close_quick_doc(&mut self, cx: &mut Context<Self>) {
        if self.quick_doc.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn render_quick_doc(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let quick = self.quick_doc.as_ref()?;
        let theme = cx.theme();
        let location = format!(
            "{}:{}",
            super::relative_path(&self.root, &quick.doc.file),
            quick.doc.row + 1
        );
        let panel = v_flex()
            .id("quick-doc")
            .absolute()
            .left(quick.at.x)
            .top(quick.at.y)
            .w(px(560.))
            .max_h(px(360.))
            .overflow_y_scroll()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px_3()
                    .py_2()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(UI_TEXT)
                    .whitespace_nowrap()
                    .children(
                        quick
                            .doc
                            .header
                            .lines()
                            .map(|line| div().child(line.replace('\t', "    "))),
                    ),
            )
            .when(!quick.doc.comment.is_empty(), |panel| {
                panel.child(
                    div()
                        .px_3()
                        .py_2()
                        .border_t_1()
                        .border_color(theme.border)
                        .text_size(UI_TEXT)
                        .children(quick.doc.comment.lines().map(|line| div().child(line.to_string()))),
                )
            })
            .child(
                div()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(theme.border)
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .child(location),
            );
        Some(
            div()
                .id("quick-doc-layer")
                .absolute()
                .inset_0()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| this.close_quick_doc(cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
