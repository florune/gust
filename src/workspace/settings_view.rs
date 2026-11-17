//! The settings page (Ctrl+Alt+S): color themes, code font size, formatting on
//! save, inline blame, hover documentation. Ctrl+wheel over an editor
//! changes the font size as well.

use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, ThemeMode, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::schemes::{SCHEMES, Scheme};
use crate::settings::{self, AppSettings, FONT_SIZES};
use crate::ui::{UI_SMALL, UI_TEXT};

actions!(gust, [OpenSettings, CloseSettings]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-alt-s", OpenSettings, Some("Workspace")),
        KeyBinding::new("escape", CloseSettings, Some("Settings")),
    ]);
}

/// Wheel movement that makes one step of font size.
const ZOOM_STEP: f32 = 40.;

impl Workspace {
    pub(super) fn on_open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.settings_page = Some(focus);
        cx.notify();
    }

    pub(super) fn on_close_settings(&mut self, _: &CloseSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    /// Change the application settings with `change`, apply and keep them.
    pub(super) fn change_app_settings(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut AppSettings)) {
        // Shortcuts are changed on their own page, maybe in another window.
        self.app.keymap = settings::load_app().keymap;
        let before = self.app.clone();
        change(&mut self.app);
        let app = self.app.clone();
        if app == before {
            return;
        }
        if app.font_size != before.font_size {
            crate::theme::set_font_size(app.font_size, cx);
        }
        let themes_changed = app.dark_theme != before.dark_theme || app.light_theme != before.light_theme;
        if themes_changed {
            crate::theme::set_themes(&app.dark_theme, &app.light_theme, cx);
        }
        if app.dark != before.dark || themes_changed {
            crate::theme::change(if app.dark { ThemeMode::Dark } else { ThemeMode::Light }, cx);
        }
        if app.dark != before.dark || app.bracket_colors != before.bracket_colors {
            self.refresh_all_brackets(cx);
        }
        crate::complete::set_hover_docs(app.hover_docs);
        if let Err(reason) = settings::save_app(&app) {
            self.notice = Some(reason.into());
        }
        cx.notify();
    }

    fn step_font_size(&mut self, steps: f32, cx: &mut Context<Self>) {
        self.change_app_settings(cx, |app| {
            app.font_size = (app.font_size + steps).clamp(*FONT_SIZES.start(), *FONT_SIZES.end());
        });
    }

    /// Ctrl+wheel over an editor: a larger or smaller code font.
    pub(super) fn zoom_wheel(&mut self, event: &ScrollEvent, cx: &mut Context<Self>) -> bool {
        if !event.modifiers.control || !self.editor_at(event.position, cx) {
            return false;
        }
        self.zoom_remainder += f32::from(event.delta.pixel_delta(px(20.)).y);
        let steps = (self.zoom_remainder / ZOOM_STEP).trunc();
        if steps != 0. {
            self.zoom_remainder -= steps * ZOOM_STEP;
            self.step_font_size(steps, cx);
        }
        true
    }

    /// A layer that sees the wheel before the editors do.
    pub(super) fn render_zoom_listener(&self, cx: &mut Context<Self>) -> AnyElement {
        let workspace = cx.entity().downgrade();
        canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                let workspace = workspace.clone();
                window.on_mouse_event(move |event: &ScrollEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || !event.modifiers.control {
                        return;
                    }
                    let zoomed = workspace
                        .update(cx, |this, cx| this.zoom_wheel(event, cx))
                        .unwrap_or(false);
                    if zoomed {
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .size_0()
        .into_any_element()
    }

    pub(super) fn render_settings_page(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let focus = self.settings_page.as_ref()?;
        let theme = cx.theme();
        let app = &self.app;
        let row = |label: &'static str, hint: &'static str, control: AnyElement| {
            h_flex()
                .py_2()
                .gap_4()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    v_flex()
                        .flex_1()
                        .child(div().text_size(UI_TEXT).child(label))
                        .when(!hint.is_empty(), |text| {
                            text.child(
                                div()
                                    .text_size(UI_SMALL)
                                    .text_color(theme.muted_foreground)
                                    .child(hint),
                            )
                        }),
                )
                .child(control)
        };
        let check = |id: &'static str, on: bool, set: fn(&mut AppSettings, bool)| {
            Checkbox::new(id)
                .checked(on)
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    let checked = *checked;
                    this.change_app_settings(cx, |app| set(app, checked))
                }))
                .into_any_element()
        };
        // A miniature of each theme: its sidebar, background, and a few lines of code.
        let card = |scheme: &'static Scheme| {
            let chosen = if scheme.dark { app.dark_theme == scheme.name } else { app.light_theme == scheme.name };
            let in_use = chosen && app.dark == scheme.dark;
            let bar = |color: u32, width: f32| div().h(px(4.)).w(px(width)).rounded(px(2.)).bg(rgb(color));
            let code = &scheme.syntax;
            v_flex()
                .id(SharedString::from(format!("theme-{}", scheme.name)))
                .w(px(124.))
                .gap_1()
                .cursor_pointer()
                .child(
                    h_flex()
                        .h(px(56.))
                        .rounded(theme.radius)
                        .border_2()
                        .border_color(if chosen { theme.ring } else { theme.border })
                        .overflow_hidden()
                        .child(div().w(px(20.)).h_full().bg(rgb(scheme.panel)))
                        .child(
                            v_flex()
                                .flex_1()
                                .h_full()
                                .p_1p5()
                                .gap(px(5.))
                                .bg(rgb(scheme.background))
                                .child(h_flex().gap_1().child(bar(code.keyword, 16.)).child(bar(code.function, 34.)))
                                .child(h_flex().gap_1().pl_2().child(bar(code.ty, 20.)).child(bar(code.string, 38.)))
                                .child(h_flex().gap_1().pl_2().child(bar(code.number, 12.)).child(bar(code.constant, 26.)))
                                .child(h_flex().pl_2().child(bar(code.comment, 52.))),
                        ),
                )
                .child(
                    div()
                        .text_size(UI_SMALL)
                        .text_color(if in_use { theme.foreground } else { theme.muted_foreground })
                        .when(in_use, |name| name.font_weight(FontWeight::SEMIBOLD))
                        .child(scheme.name),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change_app_settings(cx, |app| {
                        app.dark = scheme.dark;
                        if scheme.dark {
                            app.dark_theme = scheme.name.to_string();
                        } else {
                            app.light_theme = scheme.name.to_string();
                        }
                    })
                }))
        };
        let themes = |dark: bool| {
            h_flex()
                .flex_wrap()
                .gap_3()
                .children(SCHEMES.iter().filter(|scheme| scheme.dark == dark).map(card))
        };
        let theme_section = |title: &'static str, dark: bool| {
            v_flex()
                .py_2()
                .gap_2()
                .border_b_1()
                .border_color(theme.border)
                .child(div().text_size(UI_TEXT).child(title))
                .child(themes(dark))
        };
        let theme_hint = div()
            .pt_2()
            .text_size(UI_SMALL)
            .text_color(theme.muted_foreground)
            .child("深色与浅色各记住一个主题，活动栏底部的按钮在两者之间切换。");

        let font = h_flex()
            .gap_2()
            .child(
                Button::new("settings-font-smaller")
                    .small()
                    .label("−")
                    .on_click(cx.listener(|this, _, _, cx| this.step_font_size(-1., cx))),
            )
            .child(
                div()
                    .w(px(32.))
                    .text_center()
                    .text_size(UI_TEXT)
                    .child(format!("{}", app.font_size)),
            )
            .child(
                Button::new("settings-font-larger")
                    .small()
                    .label("+")
                    .on_click(cx.listener(|this, _, _, cx| this.step_font_size(1., cx))),
            )
            .into_any_element();

        let panel = v_flex()
            .id("settings-page")
            .key_context("Settings")
            .track_focus(focus)
            .occlude()
            .w(px(600.))
            .max_h(relative(0.85))
            .overflow_y_scroll()
            .p_4()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .pb_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(UI_TEXT)
                    .child(div().flex_1().child("设置"))
                    .child(
                        Self::header_button("settings-close", "x", "关闭 (Esc)", cx).on_click(
                            cx.listener(|this, _, window, cx| this.on_close_settings(&CloseSettings, window, cx)),
                        ),
                    ),
            )
            .child(theme_hint)
            .child(theme_section("深色主题", true))
            .child(theme_section("浅色主题", false))
            .child(row("代码字号", "在编辑器中按住 Ctrl 滚动鼠标滚轮也可调整", font))
            .child(row(
                "保存时格式化",
                "保存 Go 文件时运行 gofmt 或 goimports",
                check("settings-format", app.format_on_save, |app, on| app.format_on_save = on),
            ))
            .child(row(
                "行内 Git blame",
                "在光标所在行的末尾显示作者与提交",
                check("settings-blame", app.blame, |app, on| app.blame = on),
            ))
            .child(row(
                "悬停显示文档",
                "鼠标停留在名称上时显示声明与注释",
                check("settings-hover", app.hover_docs, |app, on| app.hover_docs = on),
            ))
            .child(row(
                "用法与实现数量",
                "在函数、方法、类型的声明行末尾显示用法数量，接口另显示实现数量",
                check("settings-lens", app.code_lens, |app, on| app.code_lens = on),
            ))
            .child(row(
                "彩色括号",
                "按嵌套层级为括号着色；光标在括号旁时标出与之配对的另一半",
                check("settings-brackets", app.bracket_colors, |app, on| app.bracket_colors = on),
            ));
        Some(
            div()
                .id("settings-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(96.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.on_close_settings(&CloseSettings, window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
