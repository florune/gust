//! The application menu behind the logo in the title bar: settings,
//! shortcuts, the configuration folder, and the About box.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::text::TextView;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::*;

use super::Workspace;
use super::keymap_view::OpenKeymap;
use super::settings_view::OpenSettings;
use crate::ui::{UI_SMALL, UI_TEXT, app_logo, icon};

actions!(gust, [ShowAbout, CloseAbout]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseAbout, Some("About"))]);
}

pub(super) const VERSION: &str = env!("CARGO_PKG_VERSION");

impl Workspace {
    pub(super) fn render_app_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let workspace = cx.entity().downgrade();
        // Shortcuts shown in the menu are those bound in the workspace.
        let focus = self.focus.clone();
        Button::new("app-menu")
            .ghost()
            .small()
            .child(app_logo(px(20.)))
            .dropdown_menu(move |menu, _, _| {
                let settings = workspace.clone();
                let keymap = workspace.clone();
                let about = workspace.clone();
                menu.action_context(focus.clone())
                    .label(format!("Gust {VERSION}"))
                    .item(
                        PopupMenuItem::new("设置…")
                            .icon(icon("settings"))
                            .action(Box::new(OpenSettings))
                            .on_click(move |_, window, cx| {
                                settings
                                    .update(cx, |this, cx| this.on_open_settings(&OpenSettings, window, cx))
                                    .ok();
                            }),
                    )
                    .item(
                        PopupMenuItem::new("快捷键…")
                            .icon(icon("keyboard"))
                            .action(Box::new(OpenKeymap))
                            .on_click(move |_, window, cx| {
                                keymap
                                    .update(cx, |this, cx| this.on_open_keymap(&OpenKeymap, window, cx))
                                    .ok();
                            }),
                    )
                    .item(
                        PopupMenuItem::new("打开配置文件夹")
                            .icon(icon("folder-open"))
                            .on_click(|_, _, _| {
                                let dir = crate::settings::config_dir();
                                let _ = std::fs::create_dir_all(&dir);
                                let _ = crate::proc::command("explorer").arg(&dir).spawn();
                            }),
                    )
                    .separator()
                    .item(PopupMenuItem::new("关于 Gust").icon(icon("info")).on_click(move |_, window, cx| {
                        about.update(cx, |this, cx| this.on_show_about(&ShowAbout, window, cx)).ok();
                    }))
            })
    }

    pub(super) fn on_show_about(&mut self, _: &ShowAbout, window: &mut Window, cx: &mut Context<Self>) {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.about = Some(focus);
        cx.notify();
    }

    pub(super) fn on_close_about(&mut self, _: &CloseAbout, window: &mut Window, cx: &mut Context<Self>) {
        if self.about.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    pub(super) fn render_about(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let focus = self.about.as_ref()?;
        let theme = cx.theme();
        let details = [
            ("版本", VERSION.to_string()),
            ("平台", format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)),
            ("配置文件夹", crate::settings::config_dir().display().to_string()),
        ];
        let mut rows = v_flex().gap_1p5().text_size(UI_TEXT);
        for (ix, (label, value)) in details.into_iter().enumerate() {
            rows = rows.child(
                h_flex()
                    .items_start()
                    .gap_3()
                    .child(div().w(px(76.)).flex_shrink_0().text_color(theme.muted_foreground).child(label))
                    .child(
                        div().flex_1().min_w_0().child(
                            // Selectable, so the folder can be copied.
                            TextView::markdown(SharedString::from(format!("about-{ix}")), format!("`{value}`"))
                                .selectable(true),
                        ),
                    ),
            );
        }
        let panel = v_flex()
            .id("about")
            .key_context("About")
            .track_focus(focus)
            .occlude()
            .w(px(460.))
            .p_5()
            .gap_3()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_action(cx.listener(Self::on_close_about))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .gap_3()
                    .child(app_logo(px(44.)))
                    .child(
                        v_flex()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Gust"),
                            )
                            .child(
                                div()
                                    .text_size(UI_SMALL)
                                    .text_color(theme.muted_foreground)
                                    .child("轻量的 Go 集成开发环境"),
                            ),
                    ),
            )
            .child(rows)
            .child(
                h_flex().justify_end().child(
                    Button::new("about-close")
                        .small()
                        .label("关闭")
                        .on_click(cx.listener(|this, _, window, cx| this.on_close_about(&CloseAbout, window, cx))),
                ),
            );
        Some(
            div()
                .id("about-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(120.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.on_close_about(&CloseAbout, window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
