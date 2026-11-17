//! The search box that drops down over the editor: files by name (Ctrl+P),
//! or text in files (Ctrl+Shift+F).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
    [
        FindFile,
        FindText,
        FindSymbol,
        ReplaceText,
        PalettePrevious,
        PaletteNext,
        PaletteConfirm,
        PaletteClose
    ]
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
        KeyBinding::new("ctrl-n", FindSymbol, workspace),
        KeyBinding::new("ctrl-shift-r", ReplaceText, workspace),
        KeyBinding::new("ctrl-alt-shift-n", FindSymbol, workspace),
        KeyBinding::new("up", PalettePrevious, palette),
        KeyBinding::new("down", PaletteNext, palette),
        KeyBinding::new("enter", PaletteConfirm, palette),
        KeyBinding::new("escape", PaletteClose, palette),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Files,
    Text,
    /// Usages found beforehand; typing narrows them.
    Usages,
    /// The declarations of the open file.
    Outline,
    /// The files opened last.
    Recent,
    /// Compile errors and vet findings.
    Problems,
    /// Declarations across the project, by name.
    Symbols,
    /// The versions of a file kept in local history.
    History,
    /// Text in files, with what to replace it by.
    Replace,
    /// Stashed changes, to bring one back.
    Stashes,
    /// What can be done at the caret.
    Intentions,
    /// Interfaces of the project, to implement one.
    Interfaces,
    /// Commands, by name.
    Actions,
}

pub(super) struct Palette {
    pub mode: Mode,
    pub input: Entity<InputState>,
    pub hits: Vec<Hit>,
    /// What the box lists, shown in its header.
    pub title: Option<SharedString>,
    /// Every usage, for [`Mode::Usages`].
    pub found: Vec<Hit>,
    /// The file whose versions [`Mode::History`] lists.
    pub history_file: Option<std::path::PathBuf>,
    /// What the text found is replaced by, for [`Mode::Replace`].
    pub replacement: Option<Entity<InputState>>,
    pub selected: usize,
    pub scroll: ScrollHandle,
    /// The search in flight; replacing it abandons the previous one.
    search: Task<()>,
    cancelled: Arc<AtomicBool>,
    _subscription: Subscription,
}

/// Two taps of Shift, as in GoLand, open the file search.
#[derive(Default)]
pub(super) struct ShiftTaps {
    /// When Shift went down alone.
    pressed: Option<Instant>,
    /// When the last tap ended, if nothing else was pressed since.
    last: Option<Instant>,
}

/// A tap is a short press, and two taps are close together.
const TAP: Duration = Duration::from_millis(350);
const BETWEEN_TAPS: Duration = Duration::from_millis(450);

impl Workspace {
    pub(super) fn open_palette(&mut self, mode: Mode, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(match mode {
                Mode::Files => "按文件名或 文件:行号 查找",
                Mode::Text => "在所有文件中查找文字",
                Mode::Usages => "筛选",
                Mode::Outline => "按名称筛选",
                Mode::Recent => "按文件名筛选",
                Mode::Problems => "筛选",
                Mode::Symbols => "按名称查找类型、函数、方法、变量",
                Mode::History => "筛选",
                Mode::Replace => "在所有文件中查找文字",
                Mode::Stashes => "筛选",
                Mode::Intentions => "筛选",
                Mode::Interfaces => "按名称筛选",
                Mode::Actions => "输入操作名称",
            })
        });
        let replacement = (mode == Mode::Replace)
            .then(|| cx.new(|cx| InputState::new(window, cx).placeholder("替换为")));
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
            history_file: None,
            replacement,
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
        if matches!(
            mode,
            Mode::Usages
                | Mode::Outline
                | Mode::Recent
                | Mode::Problems
                | Mode::History
                | Mode::Stashes
                | Mode::Intentions
                | Mode::Interfaces
                | Mode::Actions
        ) {
            let query = query.to_lowercase();
            palette.hits = palette
                .found
                .iter()
                .filter(|hit| {
                    let path = matches!(mode, Mode::Usages | Mode::Recent | Mode::Problems)
                        && hit.path.to_lowercase().contains(&query);
                    path || hit.preview.to_lowercase().contains(&query)
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
        let index = self.index.clone();
        palette.search = cx.spawn(async move |this, cx| {
            // Reading every file is worth holding off for until typing pauses.
            if matches!(mode, Mode::Text | Mode::Replace) {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
            }
            let searching = cx.background_executor().spawn(async move {
                let files =
                    known.unwrap_or_else(|| Arc::new(finder::project_files(&root, &hidden)));
                let hits = match mode {
                    Mode::Files
                    | Mode::Usages
                    | Mode::Outline
                    | Mode::Recent
                    | Mode::Problems
                    | Mode::History
                    | Mode::Stashes
                    | Mode::Intentions
                    | Mode::Interfaces
                    | Mode::Actions => {
                        finder::find_files(&files, &query, MAX_RESULTS)
                    }
                    Mode::Text | Mode::Replace => {
                        finder::find_text(&root, &files, &query, MAX_RESULTS, &cancelled)
                    }
                    Mode::Symbols => find_symbols(&index, &root, &files, &query),
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
        // An intention is carried out; an interface is implemented; a
        // command runs.
        let mode = self.palette.as_ref().map(|palette| palette.mode);
        if mode == Some(Mode::Actions) {
            let ix = hit.path.parse().unwrap_or(usize::MAX);
            self.palette = None;
            self.run_command(ix, window, cx);
            return;
        }
        if mode == Some(Mode::Intentions) {
            let id = hit.path.clone();
            self.close_palette(window, cx);
            self.run_intention(&id, window, cx);
            return;
        }
        if mode == Some(Mode::Interfaces) {
            let (file, name) = (std::path::PathBuf::from(&hit.path), hit.preview.clone());
            self.close_palette(window, cx);
            self.implement_interface(file, name, window, cx);
            return;
        }
        // A stash is brought back.
        if self.palette.as_ref().is_some_and(|palette| palette.mode == Mode::Stashes) {
            let reference = hit.path.clone();
            self.close_palette(window, cx);
            self.unstash(reference, window, cx);
            return;
        }
        // A version of local history opens in a compare view.
        if let Some(palette) = &self.palette
            && palette.mode == Mode::History
            && let Some(file) = palette.history_file.clone()
        {
            let version = std::path::PathBuf::from(&hit.path);
            self.close_palette(window, cx);
            self.compare_with_version(file, version, window, cx);
            return;
        }
        // A file found with a line opens at that line.
        let text = self
            .palette
            .as_ref()
            .is_some_and(|p| !matches!(p.mode, Mode::Files | Mode::Recent))
            || !hit.preview.is_empty();
        self.close_palette(window, cx);
        if text {
            self.open_at(path, row, col, window, cx);
        } else {
            self.open(path, window, cx);
        }
    }

    pub(super) fn shift_modifiers_changed(
        &mut self,
        event: &ModifiersChangedEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers;
        let others = modifiers.control || modifiers.alt || modifiers.platform || modifiers.function;
        let taps = &mut self.shift_taps;
        if modifiers.shift && !others {
            taps.pressed = Some(Instant::now());
            return;
        }
        let tapped = !modifiers.shift
            && !others
            && taps.pressed.take().is_some_and(|pressed| pressed.elapsed() < TAP);
        if !tapped {
            taps.pressed = None;
            taps.last = None;
            return;
        }
        if taps.last.take().is_some_and(|last| last.elapsed() < BETWEEN_TAPS) {
            if self.palette.as_ref().is_some_and(|palette| palette.mode == Mode::Files) {
                self.close_palette(window, cx);
            } else {
                self.open_palette(Mode::Files, window, cx);
            }
        } else {
            taps.last = Some(Instant::now());
        }
    }

    /// Any key between the taps makes them not a double tap; any key closes
    /// the documentation popup.
    pub(super) fn shift_key_down(&mut self, _: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.shift_taps = ShiftTaps::default();
        self.close_quick_doc(cx);
    }

    pub(super) fn on_find_file(&mut self, _: &FindFile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Files, window, cx);
    }

    pub(super) fn on_find_text(&mut self, _: &FindText, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Text, window, cx);
    }

    pub(super) fn on_find_symbol(&mut self, _: &FindSymbol, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Symbols, window, cx);
    }

    pub(super) fn on_replace_text(&mut self, _: &ReplaceText, window: &mut Window, cx: &mut Context<Self>) {
        self.open_palette(Mode::Replace, window, cx);
    }

    /// Replace the text searched for in every project file; open files change
    /// in their editors, others on disk after a local history snapshot.
    pub(super) fn replace_in_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(palette) = &self.palette else {
            return;
        };
        let query = palette.input.read(cx).value().to_string();
        let Some(replacement) = palette.replacement.as_ref().map(|input| input.read(cx).value().to_string()) else {
            return;
        };
        if query.is_empty() {
            return;
        }
        let files = self
            .project_files
            .clone()
            .unwrap_or_else(|| Arc::new(finder::project_files(&self.root, &self.settings.hidden)));
        let (mut count, mut changed) = (0, 0);
        for relative in files.iter() {
            let path = self.root.join(relative);
            if let Some(doc) = self.docs.iter().find(|doc| doc.path == path) {
                let editor = doc.editor.clone();
                let before = editor.read(cx).text().to_string();
                let (after, n) = finder::replace_all(&before, &query, &replacement);
                if n > 0 {
                    self.apply_as_one_edit(&editor, &before, &after, None, window, cx);
                    count += n;
                    changed += 1;
                }
                continue;
            }
            if !finder::searchable(&path) {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let (after, n) = finder::replace_all(&raw, &query, &replacement);
            if n == 0 {
                continue;
            }
            self.snapshot_before_change(&path, &raw.replace("\r\n", "\n"), "替换前");
            match std::fs::write(&path, after) {
                Ok(()) => {
                    count += n;
                    changed += 1;
                }
                Err(err) => self.notice = Some(format!("无法写入 {relative}：{err}").into()),
            }
        }
        self.close_palette(window, cx);
        if self.notice.is_none() {
            self.inform(format!("已替换 {count} 处，{changed} 个文件"), cx);
        }
        cx.notify();
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
                Mode::Files | Mode::Recent => (format!("{name}{}", hit.preview), folder.to_string()),
                Mode::Text | Mode::Replace => (hit.preview.clone(), format!("{name}:{}", hit.row + 1)),
                Mode::Usages | Mode::Problems | Mode::Symbols => {
                    (hit.preview.clone(), format!("{}:{}", hit.path, hit.row + 1))
                }
                Mode::Outline
                | Mode::History
                | Mode::Stashes
                | Mode::Intentions
                | Mode::Interfaces
                | Mode::Actions => {
                    (hit.preview.clone(), hit.detail.clone())
                }
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
                        .when(!matches!(palette.mode, Mode::Files | Mode::Recent), |title| {
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
                        .when(!matches!(palette.mode, Mode::Files | Mode::Recent), |detail| detail.text_right())
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
                            Mode::Replace => "replace",
                            Mode::Stashes => "archive",
                            Mode::Intentions => "lightbulb",
                            Mode::Interfaces => "braces",
                            Mode::Actions => "command",
                            Mode::Usages => "list-tree",
                            Mode::Outline => "list",
                            Mode::Recent => "clock",
                            Mode::Problems => "circle-alert",
                            Mode::Symbols => "braces",
                            Mode::History => "clock",
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
            .when_some(palette.replacement.clone(), |panel, replacement| {
                use gpui_kit::component::button::{Button, ButtonVariants as _};
                panel.child(
                    h_flex()
                        .px_3()
                        .gap_2()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(icon("replace").small().text_color(theme.muted_foreground))
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&replacement).appearance(false).bordered(false)),
                        )
                        .child(
                            Button::new("replace-all")
                                .xsmall()
                                .primary()
                                .label("全部替换")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.replace_in_files(window, cx)
                                })),
                        ),
                )
            })
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

/// Declarations named like `query` in the Go packages among `files`.
fn find_symbols(index: &crate::goindex::Index, root: &std::path::Path, files: &[String], query: &str) -> Vec<Hit> {
    let mut dirs: Vec<std::path::PathBuf> = files
        .iter()
        .filter(|file| file.ends_with(".go"))
        .filter_map(|file| root.join(file).parent().map(std::path::Path::to_path_buf))
        .collect();
    dirs.sort();
    dirs.dedup();
    crate::goindex::find_symbols(index, &dirs, query, MAX_RESULTS)
        .into_iter()
        .map(|symbol| Hit {
            path: super::relative_path(root, &symbol.file),
            row: symbol.row,
            col: symbol.col,
            preview: match &symbol.owner {
                Some(owner) => format!("{owner}.{}", symbol.name),
                None => symbol.name.clone(),
            },
            tag: super::outline::kind_label(symbol.kind),
            detail: symbol.detail,
        })
        .collect()
}
