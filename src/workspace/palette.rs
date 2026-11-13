//! The search box that drops down over the editor: files by name (Ctrl+P),
//! or text in files (Ctrl+Shift+F).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::finder::{self, Hit};
use crate::ui::icon;

const MAX_RESULTS: usize = 60;

actions!(
    palette,
    [FindFile, FindText, PalettePrevious, PaletteNext, PaletteConfirm, PaletteClose]
);

pub(super) fn init(cx: &mut App) {
    let workspace = Some("Workspace");
    // The same depth as the input's own bindings, registered after them, so
    // these win for the keys that move through the results.
    let palette = Some("Palette > Input");
    cx.bind_keys([
        KeyBinding::new("ctrl-p", FindFile, workspace),
        KeyBinding::new("ctrl-shift-n", FindFile, workspace),
        KeyBinding::new("ctrl-shift-f", FindText, workspace),
        KeyBinding::new("up", PalettePrevious, palette),
        KeyBinding::new("down", PaletteNext, palette),
        KeyBinding::new("enter", PaletteConfirm, palette),
        KeyBinding::new("escape", PaletteClose, palette),
    ]);
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Mode {
    Files,
    Text,
    /// Usages found beforehand; typing narrows them.
    Usages,
}

pub(super) struct Palette {
    pub mode: Mode,
    pub input: Entity<InputState>,
    pub hits: Vec<Hit>,
    /// What the box lists, shown in its header.
    pub title: Option<SharedString>,
    /// Every usage, for [`Mode::Usages`].
    pub found: Vec<Hit>,
    pub selected: usize,
    scroll: ScrollHandle,
    /// The search in flight; replacing it abandons the previous one.
    search: Task<()>,
    cancelled: Arc<AtomicBool>,
    _subscription: Subscription,
}

impl Workspace {
    pub(super) fn open_palette(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(match mode {
                Mode::Files => "按文件名查找",
                Mode::Text => "在所有文件中查找文字",
                Mode::Usages => "筛选",
            })
        });
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search(cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.palette = Some(Palette {
            mode,
            input,
            hits: Vec::new(),
            title: None,
            found: Vec::new(),
            selected: 0,
            scroll: ScrollHandle::new(),
            search: Task::ready(()),
            cancelled: Arc::new(AtomicBool::new(false)),
            _subscription: subscription,
        });
        self.search(cx);
        cx.notify();
    }

    pub(super) fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(palette) = self.palette.take() {
            palette.cancelled.store(true, Ordering::Relaxed);
            if self.diff_active {
                self.show_diff(window, cx);
            } else {
                self.focus_editor(window, cx);
            }
            cx.notify();
        }
    }

    /// Search for what the box holds now.
    fn search(&mut self, cx: &mut Context<Self>) {
        let Some(palette) = &mut self.palette else {
            return;
        };
        let query = palette.input.read(cx).value().to_string();
        let mode = palette.mode;
        if mode == Mode::Usages {
            let query = query.to_lowercase();
            palette.hits = palette
                .found
                .iter()
                .filter(|hit| {
                    hit.path.to_lowercase().contains(&query)
                        || hit.preview.to_lowercase().contains(&query)
                })
                .cloned()
                .collect();
            palette.selected = 0;
            palette.scroll.scroll_to_item(0);
            cx.notify();
            return;
        }
        palette.cancelled.store(true, Ordering::Relaxed);
        let cancelled = Arc::new(AtomicBool::new(false));
        palette.cancelled = cancelled.clone();

        let root = self.root.clone();
        let known = self.project_files.clone();
        let hidden = self.settings.hidden.clone();
        palette.search = cx.spawn(async move |this, cx| {
            // Reading every file is worth holding off for until typing pauses.
            if mode == Mode::Text {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
            let searching = cx.background_executor().spawn(async move {
                let files =
                    known.unwrap_or_else(|| Arc::new(finder::project_files(&root, &hidden)));
                let hits = match mode {
                    Mode::Files | Mode::Usages => finder::find_files(&files, &query, MAX_RESULTS),
                    Mode::Text => {
                        finder::find_text(&root, &files, &query, MAX_RESULTS, &cancelled)
                    }
                };
                (files, hits, cancelled)
            });
            let (files, hits, cancelled) = searching.await;
            this.update(cx, |this, cx| {
                this.project_files = Some(files);
                if let Some(palette) = &mut this.palette
                    && !cancelled.load(Ordering::Relaxed)
                {
                    palette.hits = hits;
                    palette.selected = 0;
                    palette.scroll.scroll_to_item(0);
                    cx.notify();
                }
            })
            .ok();
        });
    }

    fn move_selection(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(palette) = &mut self.palette else {
            return;
        };
        if palette.hits.is_empty() {
            return;
        }
        let count = palette.hits.len() as isize;
        palette.selected = (palette.selected as isize + by).rem_euclid(count) as usize;
        palette.scroll.scroll_to_item(palette.selected);
        cx.notify();
    }

    fn confirm_palette(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(hit) = self.palette.as_ref().and_then(|palette| palette.hits.get(ix)) else {
            return;
        };
        let (path, row, col) = (self.root.join(&hit.path), hit.row, hit.col);
        let text = self.palette.as_ref().is_some_and(|p| p.mode != Mode::Files);
        self.close_palette(window, cx);
        if text {
            self.open_at(path, row, col, window, cx);
        } else {
            self.open(path, window, cx);
        }
    }

    pub(super) fn on_find_file(&mut self, _: &FindFile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Files, window, cx);
    }

    pub(super) fn on_find_text(&mut self, _: &FindText, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Text, window, cx);
    }

    pub(super) fn on_palette_previous(
        &mut self,
        _: &PalettePrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(-1, cx);
    }

    pub(super) fn on_palette_next(&mut self, _: &PaletteNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    pub(super) fn on_palette_confirm(
        &mut self,
        _: &PaletteConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(selected) = self.palette.as_ref().map(|palette| palette.selected) {
            self.confirm_palette(selected, window, cx);
        }
    }

    pub(super) fn on_palette_close(
        &mut self,
        _: &PaletteClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_palette(window, cx);
    }

    /// The palette as a layer over the whole window.
    pub(super) fn render_palette(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let palette = self.palette.as_ref()?;
        let theme = cx.theme();
        let rows = palette.hits.iter().enumerate().map(|(ix, hit)| {
            let (folder, name) = match hit.path.rsplit_once('/') {
                Some((folder, name)) => (folder, name),
                None => ("", hit.path.as_str()),
            };
            let (title, detail) = match palette.mode {
                Mode::Files => (name.to_string(), folder.to_string()),
                Mode::Text => (hit.preview.clone(), format!("{name}:{}", hit.row + 1)),
                Mode::Usages => (hit.preview.clone(), format!("{}:{}", hit.path, hit.row + 1)),
            };
            h_flex()
                .id(("palette-row", ix))
                .h(px(28.))
                .flex_shrink_0()
                .px_3()
                .gap_3()
                .text_sm()
                .rounded(px(4.))
                .when(ix == palette.selected, |row| row.bg(theme.list_active))
                .hover(|row| row.bg(theme.list_hover))
                .when(!hit.tag.is_empty(), |row| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .px_1p5()
                            .rounded(px(3.))
                            .text_xs()
                            .bg(theme.primary.opacity(0.18))
                            .text_color(theme.primary)
                            .child(hit.tag),
                    )
                })
                .child(
                    div()
                        .flex_shrink_0()
                        .max_w(px(420.))
                        .truncate()
                        .when(palette.mode == Mode::Usages, |title| {
                            title.font_family(theme.mono_font_family.clone())
                        })
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .when(palette.mode != Mode::Files, |detail| detail.text_right())
                        .child(detail),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.confirm_palette(ix, window, cx)
                }))
        });

        let panel = v_flex()
            .id("palette")
            .key_context("Palette")
            .occlude()
            .w(px(640.))
            .max_w_full()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .child(
                h_flex()
                    .px_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        icon(match palette.mode {
                            Mode::Files => "file-search",
                            Mode::Text => "search",
                            Mode::Usages => "list-tree",
                        })
                        .small()
                        .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&palette.input).appearance(false).bordered(false)),
                    )
                    .when_some(palette.title.clone(), |header, title| {
                        header.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(title),
                        )
                    }),
            )
            .child(
                v_flex()
                    .id("palette-results")
                    .track_scroll(&palette.scroll)
                    .max_h(px(392.))
                    .overflow_y_scroll()
                    .p_1()
                    .children(rows)
                    .when(palette.hits.is_empty(), |list| {
                        list.child(
                            div()
                                .px_3()
                                .py_2()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child("没有结果"),
                        )
                    }),
            );

        Some(
            div()
                .id("palette-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(72.))
                // A click outside the box dismisses it.
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_palette(window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
