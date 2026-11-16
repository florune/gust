//! Opening another project: in this window, or in a new one beside it, as
//! GoLand asks.

use std::path::PathBuf;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, TitleBar, h_flex, v_flex};
use gpui_kit::*;

use super::Workspace;
use crate::ui::{UI_SMALL, UI_TEXT};

actions!(gust, [OpenHere, OpenInNewWindow, CancelOpen]);

pub(super) fn init(cx: &mut App) {
    let choice = Some("ProjectChoice");
    cx.bind_keys([
        KeyBinding::new("enter", OpenHere, choice),
        KeyBinding::new("escape", CancelOpen, choice),
    ]);
}

/// Open a window on the project at `root`, with `files` open in it.
pub fn open_window(root: PathBuf, files: Vec<PathBuf>, cx: &mut App) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1280.), px(820.)),
            cx,
        ))),
        window_min_size: Some(size(px(640.), px(400.))),
        ..TitleBar::window_options()
    };
    let opened = gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| {
            let mut workspace = Workspace::new(root, window, cx);
            for file in files {
                workspace.open(file, window, cx);
            }
            workspace
        })
    });
    if let Err(err) = opened {
        eprintln!("无法打开窗口：{err}");
    }
}

/// The project waiting for where to open it, and what has the keyboard meanwhile.
pub(super) struct ProjectChoice {
    pub root: PathBuf,
    focus: FocusHandle,
}

impl Workspace {
    /// Open the project at `root`, asking first whether here or in a new window.
    pub(super) fn choose_project(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if root == self.root {
            return;
        }
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.project_choice = Some(ProjectChoice { root, focus });
        cx.notify();
    }

    fn close_project_choice(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<PathBuf> {
        let choice = self.project_choice.take()?;
        self.focus_editor(window, cx);
        cx.notify();
        Some(choice.root)
    }

    pub(super) fn on_open_here(&mut self, _: &OpenHere, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(root) = self.close_project_choice(window, cx) {
            self.set_root(root, window, cx);
        }
    }

    pub(super) fn on_open_in_new_window(
        &mut self,
        _: &OpenInNewWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(root) = self.close_project_choice(window, cx) {
            // After this update: a window is not opened from inside another's.
            cx.defer(move |cx| open_window(root, Vec::new(), cx));
        }
    }

    pub(super) fn on_cancel_open(&mut self, _: &CancelOpen, window: &mut Window, cx: &mut Context<Self>) {
        self.close_project_choice(window, cx);
    }

    pub(super) fn render_project_choice(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let choice = self.project_choice.as_ref()?;
        let theme = cx.theme();
        let name = choice
            .root
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| choice.root.to_string_lossy().into_owned());
        let panel = v_flex()
            .id("project-choice")
            .key_context("ProjectChoice")
            .track_focus(&choice.focus)
            .occlude()
            .w(px(440.))
            .p_4()
            .gap_3()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .text_size(UI_TEXT)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("打开项目 {name}")))
            .child(
                div()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .child(choice.root.to_string_lossy().into_owned()),
            )
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("open-cancel")
                            .small()
                            .label("取消")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_cancel_open(&CancelOpen, window, cx)
                            })),
                    )
                    .child(
                        Button::new("open-new-window")
                            .small()
                            .label("新窗口")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_open_in_new_window(&OpenInNewWindow, window, cx)
                            })),
                    )
                    .child(
                        Button::new("open-here")
                            .small()
                            .primary()
                            .label("当前窗口")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_open_here(&OpenHere, window, cx)
                            })),
                    ),
            );
        Some(
            div()
                .id("project-choice-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.on_cancel_open(&CancelOpen, window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
