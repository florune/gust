//! Markdown files read as documents: a rendered preview beside or instead
//! of the source, a search of the preview, and a preview that follows the
//! source as it scrolls.

use std::cell::Cell;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::text::{
    RangeHighlight, RenderedText, TextView, TextViewState, TextViewStyle,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::theme::palette;
use crate::ui::{UI_SMALL, UI_TEXT, icon};

actions!(
    markdown,
    [FindInPreview, NextMatch, PreviousMatch, CloseSearch]
);

pub(super) fn init(cx: &mut App) {
    let search = Some("PreviewSearch > Input");
    cx.bind_keys([
        // The editor binds Ctrl+F deeper down, so source mode keeps its own search.
        KeyBinding::new("ctrl-f", FindInPreview, Some("Workspace")),
        KeyBinding::new("enter", NextMatch, search),
        KeyBinding::new("f3", NextMatch, search),
        KeyBinding::new("shift-enter", PreviousMatch, search),
        KeyBinding::new("shift-f3", PreviousMatch, search),
        KeyBinding::new("escape", CloseSearch, search),
    ]);
}

/// How a Markdown file is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DocView {
    Edit,
    Split,
    Preview,
}

/// Width the text keeps in the preview, so lines stay readable on a wide window.
const READING_WIDTH: f32 = 880.;

/// The rendered side of a Markdown file.
pub(super) struct Preview {
    state: Entity<TextViewState>,
    /// The line each top-level block starts on: the preview's list items.
    block_lines: Vec<usize>,
    lines: usize,
    /// The editor's scroll position when the preview last followed it.
    followed: Option<Pixels>,
    search: Option<PreviewSearch>,
    /// Width of the pane the preview is in, measured as it is laid out.
    pane_width: Rc<Cell<Pixels>>,
    _observation: Subscription,
}

pub(super) struct PreviewSearch {
    input: Entity<InputState>,
    /// Matches in the rendered text.
    pub matches: Vec<Range<usize>>,
    pub current: usize,
    /// The rendered text the matches were found in.
    searched: Option<RenderedText>,
    _subscription: Subscription,
}

/// The lines the top-level blocks of `source` start on, zero-based, parsed the
/// way the preview parses it so that block `n` is the preview's item `n`.
pub(super) fn block_lines(source: &str) -> Vec<usize> {
    let mut options = markdown::ParseOptions::gfm();
    options.constructs.math_text = true;
    options.constructs.math_flow = true;
    match markdown::to_mdast(source, &options) {
        Ok(markdown::mdast::Node::Root(root)) => root
            .children
            .iter()
            .map(|node| node.position().map_or(0, |p| p.start.line.saturating_sub(1)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Where the preview should be for the source scrolled to `top_line`: the
/// block holding that line, and how far into the block it is.
pub(super) fn follow(block_lines: &[usize], lines: usize, top_line: f32) -> Option<(usize, f32)> {
    if block_lines.is_empty() {
        return None;
    }
    let block = block_lines
        .partition_point(|&line| line as f32 <= top_line)
        .saturating_sub(1);
    let start = block_lines[block] as f32;
    let end = block_lines.get(block + 1).copied().unwrap_or(lines.max(1)) as f32;
    let fraction = ((top_line - start) / (end - start).max(1.)).clamp(0., 1.);
    Some((block, fraction))
}

/// Byte ranges of `query` in `text`, ignoring ASCII case.
pub(super) fn find_all(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    // ASCII lowercasing keeps every byte where it was.
    let haystack = text.to_ascii_lowercase();
    let needle = query.to_ascii_lowercase();
    haystack
        .match_indices(&needle)
        .map(|(at, found)| at..at + found.len())
        .collect()
}

impl Preview {
    /// The preview of `text`, the content of `editor`.
    pub(super) fn new(
        text: &str,
        editor: &Entity<EditorState>,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let state = cx.new(|cx| TextViewState::markdown(text, cx).selectable(true));
        let editor = editor.clone();
        // A new parse brings new rendered text: the open search looks again.
        let observation = cx.observe(&state, move |this, _, cx| {
            if let Some(ix) = this.docs.iter().position(|doc| doc.editor == editor) {
                this.search_again_if_changed(ix, cx);
            }
        });
        Self {
            state,
            block_lines: block_lines(text),
            lines: text.lines().count(),
            followed: None,
            search: None,
            pane_width: Rc::new(Cell::new(px(0.))),
            _observation: observation,
        }
    }

    /// The width the text view laid itself out at in the last frame.
    #[cfg(test)]
    pub(super) fn text_width(&self, cx: &App) -> Pixels {
        self.state.read(cx).bounds().size.width
    }

    #[cfg(test)]
    pub(super) fn search(&self) -> Option<&PreviewSearch> {
        self.search.as_ref()
    }
}

impl Workspace {
    pub(super) fn set_doc_view(
        &mut self,
        ix: usize,
        view: DocView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        doc.view = view;
        if let Some(preview) = &mut doc.preview {
            preview.followed = None;
        }
        // The next Markdown file opens the way this one was last shown.
        self.markdown_view = view;
        let editor = doc.editor.clone();
        match view {
            DocView::Preview => self.focus.focus(window, cx),
            _ => editor.update(cx, |editor, cx| editor.focus(window, cx)),
        }
        self.follow_source(ix, cx);
        cx.notify();
    }

    /// Show the preview the source as it is now.
    pub(super) fn update_preview(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        let Some(preview) = &mut doc.preview else {
            return;
        };
        let text = doc.editor.read(cx).text().to_string();
        preview.block_lines = block_lines(&text);
        preview.lines = text.lines().count();
        preview
            .state
            .update(cx, |state, cx| state.set_text(&text, cx));
    }

    /// Look for the query again when the preview shows different text,
    /// keeping the current match where it was and the view where it is.
    fn search_again_if_changed(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(preview) = self.docs.get(ix).and_then(|doc| doc.preview.as_ref()) else {
            return;
        };
        let Some(search) = &preview.search else {
            return;
        };
        if search.searched.as_ref() == Some(&preview.state.read(cx).rendered_text()) {
            return;
        }
        self.run_search(ix, false, cx);
    }

    /// In the split view, scroll the preview to where the source is scrolled.
    pub(super) fn follow_source(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        if doc.view != DocView::Split {
            return;
        }
        let Some(preview) = &mut doc.preview else {
            return;
        };
        let editor = doc.editor.read(cx);
        let scrolled = -editor.scroll_offset().y;
        if preview.followed == Some(scrolled) {
            return;
        }
        let Some(line_height) = editor.line_height().filter(|h| *h > px(0.)) else {
            return;
        };
        let list = preview.state.read(cx).list_state().clone();
        // The preview has not taken in the source's latest blocks yet.
        if list.item_count() != preview.block_lines.len() {
            return;
        }
        let top_line = scrolled / line_height;
        let Some((block, fraction)) = follow(&preview.block_lines, preview.lines, top_line) else {
            return;
        };
        let height = list
            .bounds_for_item(block)
            .map_or(px(0.), |bounds| bounds.size.height);
        list.scroll_to(ListOffset {
            item_ix: block,
            offset_in_item: height * fraction,
        });
        preview.followed = Some(scrolled);
        preview.state.update(cx, |_, cx| cx.notify());
    }

    pub(super) fn on_find_in_preview(
        &mut self,
        _: &FindInPreview,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.active.filter(|_| !self.diff_active) else {
            return;
        };
        let doc = &mut self.docs[ix];
        if doc.view == DocView::Edit {
            return;
        }
        let Some(preview) = &mut doc.preview else {
            return;
        };
        let input = match &preview.search {
            Some(search) => search.input.clone(),
            None => {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("在预览中查找"));
                let editor = doc.editor.clone();
                let subscription = cx.subscribe(&input, move |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change)
                        && let Some(ix) = this.docs.iter().position(|doc| doc.editor == editor)
                    {
                        this.run_search(ix, true, cx);
                    }
                });
                preview.search = Some(PreviewSearch {
                    input: input.clone(),
                    matches: Vec::new(),
                    current: 0,
                    searched: None,
                    _subscription: subscription,
                });
                input
            }
        };
        // A word selected in the preview is what to look for.
        let selected = preview.state.read(cx).selected_text();
        let selected = selected.trim();
        if !selected.is_empty() && !selected.contains('\n') && selected.len() < 200 {
            let selected = selected.to_string();
            input.update(cx, |input, cx| input.set_value(selected, window, cx));
            self.run_search(ix, true, cx);
        }
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        cx.notify();
    }

    /// Find the query in the preview. A new query starts at the first match
    /// and shows it; new text keeps the current match and the view.
    fn run_search(&mut self, ix: usize, new_query: bool, cx: &mut Context<Self>) {
        let Some(preview) = self.docs.get_mut(ix).and_then(|doc| doc.preview.as_mut()) else {
            return;
        };
        let Some(search) = &mut preview.search else {
            return;
        };
        let query = search.input.read(cx).value().to_string();
        let text = preview.state.read(cx).rendered_text();
        search.matches = find_all(text.as_str(), query.trim());
        search.searched = Some(text);
        search.current = if new_query {
            0
        } else {
            search.current.min(search.matches.len().saturating_sub(1))
        };
        self.show_matches(ix, new_query, cx);
    }

    /// Paint the matches, the current one stronger, and bring it into view.
    fn show_matches(&mut self, ix: usize, reveal: bool, cx: &mut Context<Self>) {
        let colors = palette(cx);
        let Some(preview) = self.docs.get_mut(ix).and_then(|doc| doc.preview.as_mut()) else {
            return;
        };
        let Some(search) = &preview.search else {
            return;
        };
        let highlights: Vec<RangeHighlight> = search
            .matches
            .iter()
            .enumerate()
            .map(|(n, range)| {
                let color = if n == search.current {
                    colors.orange.opacity(0.65)
                } else {
                    colors.yellow.opacity(0.3)
                };
                RangeHighlight::new(range.clone(), color)
            })
            .collect();
        let current = search.matches.get(search.current).cloned();
        preview.state.update(cx, |state, cx| {
            if state.set_range_highlights(highlights, cx).is_err() {
                state.clear_range_highlights(cx);
            }
            if reveal && let Some(range) = current {
                let _ = state.reveal_range(range, cx);
            }
        });
        cx.notify();
    }

    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(ix) = self.active else {
            return;
        };
        let Some(search) = self
            .docs
            .get_mut(ix)
            .and_then(|doc| doc.preview.as_mut())
            .and_then(|preview| preview.search.as_mut())
        else {
            return;
        };
        let count = search.matches.len();
        if count == 0 {
            return;
        }
        search.current = if forward {
            (search.current + 1) % count
        } else {
            (search.current + count - 1) % count
        };
        self.show_matches(ix, true, cx);
    }

    pub(super) fn on_next_match(&mut self, _: &NextMatch, _: &mut Window, cx: &mut Context<Self>) {
        self.step_match(true, cx);
    }

    pub(super) fn on_previous_match(
        &mut self,
        _: &PreviousMatch,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_match(false, cx);
    }

    pub(super) fn on_close_search(
        &mut self,
        _: &CloseSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.active else {
            return;
        };
        if let Some(preview) = self.docs.get_mut(ix).and_then(|doc| doc.preview.as_mut()) {
            preview.search = None;
            preview
                .state
                .update(cx, |state, cx| state.clear_range_highlights(cx));
        }
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn render_search_bar(&self, search: &PreviewSearch, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let count = if search.matches.is_empty() {
            if search.input.read(cx).value().is_empty() {
                String::new()
            } else {
                "无结果".to_string()
            }
        } else {
            format!("{}/{}", search.current + 1, search.matches.len())
        };
        let button = |id: &'static str, glyph: &'static str, tip: &'static str| {
            div()
                .id(id)
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .text_color(theme.muted_foreground)
                .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
                .child(icon(glyph).small())
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        };
        h_flex()
            .key_context("PreviewSearch")
            .h(px(38.))
            .flex_shrink_0()
            .px_3()
            .gap_1()
            .bg(theme.sidebar)
            .border_b_1()
            .border_color(theme.border)
            .child(
                div().w(px(320.)).child(
                    Input::new(&search.input)
                        .small()
                        .prefix(icon("search").xsmall().text_color(theme.muted_foreground)),
                ),
            )
            .child(
                div()
                    .min_w(px(56.))
                    .px_1()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .child(count),
            )
            .child(
                button("search-previous", "arrow-up", "上一个 (Shift+Enter)")
                    .on_click(cx.listener(|this, _, _, cx| this.step_match(false, cx))),
            )
            .child(
                button("search-next", "arrow-down", "下一个 (Enter)")
                    .on_click(cx.listener(|this, _, _, cx| this.step_match(true, cx))),
            )
            .child(div().flex_1())
            .child(button("search-close", "x", "关闭 (Esc)").on_click(cx.listener(
                |this, _, window, cx| this.on_close_search(&CloseSearch, window, cx),
            )))
            .into_any_element()
    }

    fn render_preview(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let doc = &self.docs[ix];
        let Some(preview) = &doc.preview else {
            return div().into_any_element();
        };
        let search = preview.search.as_ref().map(|search| self.render_search_bar(search, cx));
        let theme = cx.theme();
        let dir: PathBuf = doc.path.parent().map(PathBuf::from).unwrap_or_default();

        let mut style = TextViewStyle::default()
            .paragraph_gap(rems(0.9))
            .heading_font_size(|level, _| {
                px(match level {
                    1 => 30.,
                    2 => 24.,
                    3 => 20.,
                    4 => 17.,
                    _ => 15.,
                })
            })
            .code_block(
                StyleRefinement::default()
                    .rounded(px(8.))
                    .p_4()
                    .bg(theme.sidebar)
                    .border_1()
                    .border_color(theme.border),
            )
            .table(StyleRefinement::default().rounded(px(6.)));
        style.highlight_theme = theme.highlight_theme.clone();
        style.is_dark = theme.is_dark();

        // The text keeps a reading width in the middle of a wide pane. The
        // width is the pane's, measured below, and not the text view's own:
        // that one shrinks by the margins set here, which would then shrink
        // the margins, which would widen it again, every frame.
        let side = ((preview.pane_width.get() - px(READING_WIDTH)) / 2.).max(px(40.));
        let pane_width = preview.pane_width.clone();
        let measure = canvas(
            move |bounds, window, _| {
                if (pane_width.get() - bounds.size.width).abs() > px(0.5) {
                    pane_width.set(bounds.size.width);
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .inset_0();

        v_flex()
            .size_full()
            .bg(theme.background)
            .children(search)
            .child(
                div().relative().flex_1().min_h_0().child(measure).child(
                    TextView::new(&preview.state)
                        .scrollable(true)
                        .selectable(true)
                        .style(style)
                        .size_full()
                        .px(side)
                        .pt(px(28.))
                        .text_size(px(15.))
                        .line_height(rems(1.75))
                        .text_color(theme.foreground)
                        .inline_code_style(
                            StyleRefinement::default()
                                .bg(theme.secondary)
                                .rounded(px(4.))
                                .px_1(),
                        )
                        // Pictures next to the document load from disk.
                        .image_source(move |uri| {
                            let link = uri.to_string();
                            if link.contains("://") || link.starts_with("data:") {
                                ImageSource::from(uri.clone())
                            } else {
                                ImageSource::from(dir.join(link.trim_start_matches("./")))
                            }
                        }),
                ),
            )
            .into_any_element()
    }

    /// The source / split / preview switch, shown at the end of the tab bar.
    pub(super) fn render_view_switch(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let ix = self.active.filter(|_| !self.diff_active)?;
        let doc = self.docs.get(ix).filter(|doc| doc.language == "markdown")?;
        let view = doc.view;
        let theme = cx.theme();
        let button = |id: &'static str, glyph: &'static str, tip: &'static str, this_view: DocView| {
            let active = view == this_view;
            div()
                .id(id)
                .w(px(28.))
                .h(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .when(active, |this| this.bg(theme.secondary_hover))
                .when(!active, |this| this.hover(|this| this.bg(theme.list_hover)))
                .child(icon(glyph).small())
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.set_doc_view(ix, this_view, window, cx)
                }))
        };
        let searchable = view != DocView::Edit;
        Some(
            h_flex()
                .flex_shrink_0()
                .px_2()
                .gap_0p5()
                .text_size(UI_TEXT)
                .when(searchable, |this| {
                    this.child(
                        div()
                            .id("preview-find")
                            .w(px(28.))
                            .h(px(24.))
                            .mr_1()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(5.))
                            .text_color(theme.muted_foreground)
                            .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
                            .child(icon("search").small())
                            .tooltip(|window, cx| {
                                Tooltip::new("在预览中查找 (Ctrl+F)").build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_find_in_preview(&FindInPreview, window, cx)
                            })),
                    )
                })
                .child(button("view-edit", "code", "只看源码", DocView::Edit).test_support())
                .child(button("view-split", "columns-2", "源码和预览", DocView::Split).test_support())
                .child(button("view-preview", "eye", "只看预览", DocView::Preview).test_support())
                .into_any_element(),
        )
    }

    /// The editor area for an open file: the editor, or for Markdown, the
    /// editor and its preview as the file is set to show them.
    pub(super) fn render_doc(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let doc = &self.docs[ix];
        let editor = |state: &Entity<EditorState>| {
            Editor::new(state)
                .appearance(false)
                .bordered(false)
                .size_full()
        };
        if doc.language != "markdown" {
            return div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(editor(&doc.editor))
                .children(self.render_blame_layer(ix, cx))
                .into_any_element();
        }
        let body = match doc.view {
            DocView::Edit => div()
                .relative()
                .size_full()
                .child(editor(&doc.editor))
                .children(self.render_blame_layer(ix, cx))
                .into_any_element(),
            DocView::Preview => self.render_preview(ix, cx),
            DocView::Split => h_flex()
                .size_full()
                .child(div().flex_1().min_w_0().h_full().child(editor(&doc.editor)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .border_l_1()
                        .border_color(cx.theme().border)
                        .child(self.render_preview(ix, cx)),
                )
                .into_any_element(),
        };
        div().flex_1().min_h_0().child(body).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{block_lines, find_all, follow};

    #[test]
    fn blocks_start_where_the_source_says() {
        let source = "# Title\n\nFirst paragraph\nstill first.\n\n```go\ncode\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n";
        assert_eq!(block_lines(source), [0, 2, 5, 9]);
    }

    #[test]
    fn the_preview_follows_into_the_block_at_the_top() {
        let blocks = [0, 2, 5, 9];
        assert_eq!(follow(&blocks, 12, 0.), Some((0, 0.)));
        assert_eq!(follow(&blocks, 12, 3.5), Some((1, 0.5)));
        assert_eq!(follow(&blocks, 12, 10.5), Some((3, 0.5)));
        assert_eq!(follow(&[], 12, 3.), None);
    }

    #[test]
    fn matches_ignore_case_and_keep_byte_positions() {
        assert_eq!(find_all("Snapshot 快照 snapshot", "SNAP"), [0..4, 16..20]);
        assert!(find_all("abc", "").is_empty());
    }
}
