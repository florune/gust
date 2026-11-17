//! The shortcuts page: every command with its keys, which the user can
//! change. Chosen keys are kept in the application settings and bound in
//! every window; a command left alone keeps its default keys.

use std::collections::BTreeMap;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use super::commands::{self, Command, binding_of, display_key};
use crate::settings;
use crate::ui::{UI_SMALL, UI_TEXT, icon};

actions!(gust, [OpenKeymap, CloseKeymap]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", CloseKeymap, Some("Keymap"))]);
}

pub(super) struct KeymapPage {
    focus: FocusHandle,
    filter: Entity<InputState>,
    /// The command whose next key is being recorded, and what records it.
    recording: Option<(&'static str, Subscription)>,
    _filter_changed: Subscription,
}

/// `keymap` with the entries that only repeat a command's defaults left out.
fn without_defaults(mut keymap: BTreeMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let none = BTreeMap::new();
    for command in commands::all() {
        if keymap.get(command.id).is_some_and(|keys| *keys == command.keys(&none)) {
            keymap.remove(command.id);
        }
    }
    keymap
}

/// Give `key` to command `id`, taking it from any other command that had it.
/// Returns the commands it was taken from.
fn assign(keymap: &mut BTreeMap<String, Vec<String>>, id: &str, key: &str) -> Vec<&'static str> {
    let mut taken = Vec::new();
    for command in commands::all() {
        let mut keys = command.keys(keymap);
        if command.id == id {
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.to_string());
                keymap.insert(id.to_string(), keys);
            }
        } else if keys.iter().any(|existing| existing == key) {
            keys.retain(|existing| existing != key);
            keymap.insert(command.id.to_string(), keys);
            taken.push(command.label);
        }
    }
    taken
}

impl Workspace {
    pub(super) fn on_open_keymap(&mut self, _: &OpenKeymap, window: &mut Window, cx: &mut Context<Self>) {
        self.settings_page = None;
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder("按名称或按键筛选"));
        let changed = cx.subscribe(&filter, |_, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                cx.notify();
            }
        });
        filter.update(cx, |input, cx| input.focus(window, cx));
        self.keymap_page = Some(KeymapPage {
            focus: cx.focus_handle(),
            filter,
            recording: None,
            _filter_changed: changed,
        });
        cx.notify();
    }

    /// Escape stops recording a key, or closes the page.
    pub(super) fn on_close_keymap(&mut self, _: &CloseKeymap, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = &mut self.keymap_page
            && page.recording.take().is_some()
        {
            cx.notify();
            return;
        }
        if self.keymap_page.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    /// The next key pressed becomes a shortcut of command `id`.
    pub(super) fn start_recording(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(page) = &mut self.keymap_page else {
            return;
        };
        let workspace = cx.entity().downgrade();
        let this_window = window.window_handle();
        // Seen before any binding, so the key does not also run its command.
        let interceptor = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != this_window {
                return;
            }
            let Some(key) = binding_of(&event.keystroke) else {
                return;
            };
            cx.stop_propagation();
            workspace.update(cx, |this, cx| this.record_key(&key, cx)).ok();
        });
        page.focus.focus(window, cx);
        page.recording = Some((id, interceptor));
        cx.notify();
    }

    pub(super) fn record_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some((id, _)) = self.keymap_page.as_mut().and_then(|page| page.recording.take()) else {
            return;
        };
        cx.notify();
        if key == "escape" {
            return;
        }
        let mut taken = Vec::new();
        self.change_keymap(cx, |keymap| taken = assign(keymap, id, key));
        if !taken.is_empty() {
            let from: Vec<String> = taken.iter().map(|label| format!("「{label}」")).collect();
            self.inform(format!("{} 已从{}移除", display_key(key), from.join("、")), cx);
        }
    }

    pub(super) fn remove_key(&mut self, id: &'static str, key: String, cx: &mut Context<Self>) {
        self.change_keymap(cx, |keymap| {
            let Some(command) = commands::all().into_iter().find(|command| command.id == id) else {
                return;
            };
            let mut keys = command.keys(keymap);
            keys.retain(|existing| *existing != key);
            keymap.insert(id.to_string(), keys);
        });
    }

    fn reset_command(&mut self, id: &'static str, cx: &mut Context<Self>) {
        self.change_keymap(cx, |keymap| {
            keymap.remove(id);
        });
    }

    pub(super) fn reset_keymap(&mut self, cx: &mut Context<Self>) {
        self.change_keymap(cx, BTreeMap::clear);
    }

    /// Change the keymap with `change`, bind the keys and keep them. The
    /// file is read first: another window may have changed it.
    pub(super) fn change_keymap(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut BTreeMap<String, Vec<String>>),
    ) {
        let previous = settings::load_app().keymap;
        let mut keymap = previous.clone();
        change(&mut keymap);
        let keymap = without_defaults(keymap);
        commands::apply_keymap(&keymap, &previous, cx);
        self.app.keymap = keymap;
        if let Err(reason) = settings::save_app(&self.app) {
            self.notice = Some(reason.into());
        }
        cx.notify();
    }

    pub(super) fn render_keymap_page(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let page = self.keymap_page.as_ref()?;
        let theme = cx.theme();
        let keymap = &self.app.keymap;
        let filter = page.filter.read(cx).value().trim().to_lowercase();
        let recording = page.recording.as_ref().map(|(id, _)| *id);
        let shown: Vec<Command> = commands::all()
            .into_iter()
            .filter(|command| {
                filter.is_empty()
                    || command.label.to_lowercase().contains(&filter)
                    || command.keys(keymap).iter().any(|key| {
                        display_key(key).to_lowercase().contains(&filter) || key.contains(&filter)
                    })
            })
            .collect();

        let chip = |content: AnyElement| {
            h_flex()
                .h(px(22.))
                .px_1p5()
                .gap_1()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .text_size(UI_SMALL)
                .child(content)
        };
        let mut rows = Vec::new();
        for command in &shown {
            let id = command.id;
            let customized = keymap.contains_key(id);
            let mut keys = h_flex().gap_1().flex_wrap().justify_end();
            for key in command.keys(keymap) {
                let shown = display_key(&key);
                keys = keys.child(
                    chip(div().child(shown).into_any_element()).child(
                        div()
                            .id(SharedString::from(format!("keymap-remove-{id}-{key}")))
                            .cursor_pointer()
                            .text_color(theme.muted_foreground)
                            .hover(|style| style.text_color(theme.danger))
                            .child("×")
                            .tooltip(|window, cx| Tooltip::new("移除此快捷键").build(window, cx))
                            .on_click(cx.listener(move |this, _, _, cx| this.remove_key(id, key.clone(), cx))),
                    ),
                );
            }
            if recording == Some(id) {
                keys = keys.child(
                    h_flex()
                        .h(px(22.))
                        .px_1p5()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.primary)
                        .text_size(UI_SMALL)
                        .text_color(theme.primary)
                        .child("按下快捷键…  Esc 取消"),
                );
            }
            rows.push(
                h_flex()
                    .id(SharedString::from(format!("keymap-row-{id}")))
                    .min_h(px(34.))
                    .px_2()
                    .gap_3()
                    .border_b_1()
                    .border_color(theme.border)
                    .when(recording == Some(id), |row| row.bg(theme.accent))
                    .child(
                        h_flex()
                            .flex_1()
                            .gap_2()
                            .text_size(UI_TEXT)
                            .child(command.label)
                            .when(customized, |label| {
                                label.child(
                                    div()
                                        .text_size(UI_SMALL)
                                        .text_color(theme.muted_foreground)
                                        .child("已修改"),
                                )
                            }),
                    )
                    .child(keys)
                    .child(
                        Button::new(SharedString::from(format!("keymap-add-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(icon("plus"))
                            .tooltip("添加快捷键")
                            .on_click(cx.listener(move |this, _, window, cx| this.start_recording(id, window, cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("keymap-reset-{id}")))
                            .ghost()
                            .xsmall()
                            .icon(icon("rotate-ccw"))
                            .tooltip("恢复默认")
                            .when(!customized, |button| button.invisible())
                            .on_click(cx.listener(move |this, _, _, cx| this.reset_command(id, cx))),
                    )
                    .into_any_element(),
            );
        }
        if rows.is_empty() {
            rows.push(
                div()
                    .p_4()
                    .text_size(UI_TEXT)
                    .text_color(theme.muted_foreground)
                    .child("没有匹配的命令")
                    .into_any_element(),
            );
        }

        let panel = v_flex()
            .id("keymap-page")
            .key_context("Keymap")
            .track_focus(&page.focus)
            .occlude()
            .w(px(640.))
            .h(px(560.))
            .p_4()
            .gap_2()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_action(cx.listener(Self::on_close_keymap))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .gap_2()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(UI_TEXT)
                    .child(div().flex_1().child("快捷键"))
                    .child(
                        Button::new("keymap-reset-all")
                            .small()
                            .label("全部恢复默认")
                            .disabled(keymap.is_empty())
                            .on_click(cx.listener(|this, _, _, cx| this.reset_keymap(cx))),
                    )
                    .child(
                        Self::header_button("keymap-close", "x", "关闭 (Esc)", cx).on_click(
                            cx.listener(|this, _, window, cx| this.on_close_keymap(&CloseKeymap, window, cx)),
                        ),
                    ),
            )
            .child(Input::new(&page.filter).small())
            .child(
                div()
                    .id("keymap-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(
                div()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .child("点击 + 后按下新的组合键；已被其他命令使用的按键会从原命令移除。"),
            );
        Some(
            div()
                .id("keymap-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(72.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.on_close_keymap(&CloseKeymap, window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{assign, without_defaults};

    #[test]
    fn a_key_moves_to_the_command_it_is_given_to() {
        let mut keymap = BTreeMap::new();
        let taken = assign(&mut keymap, "find_symbol", "ctrl-p");
        assert_eq!(taken, ["查找文件"]);
        assert_eq!(keymap["find_symbol"], ["ctrl-n", "ctrl-alt-shift-n", "ctrl-p"]);
        assert_eq!(keymap["find_file"], ["ctrl-shift-n"]);
    }

    #[test]
    fn keys_equal_to_the_defaults_are_not_kept() {
        let mut keymap = BTreeMap::new();
        keymap.insert("save".to_string(), vec!["ctrl-s".to_string()]);
        keymap.insert("build".to_string(), vec!["f9".to_string()]);
        let kept = without_defaults(keymap);
        assert_eq!(kept.keys().collect::<Vec<_>>(), ["build"]);
    }
}
