//! Brackets in color by how deeply they are nested, and the pair at the
//! caret marked on both ends.

use std::rc::Rc;

use gpui_kit::component::input::{TextDecoration, TextDecorationCollection};
use gpui_kit::component::{ActiveTheme as _, RopeExt as _};
use gpui_kit::*;

use super::Workspace;
use crate::brackets::Brackets;

/// Colors of the nesting levels, in turn, and of a bracket without a partner.
fn colors(dark: bool) -> ([Hsla; 3], Hsla) {
    if dark {
        ([rgb(0xffd700).into(), rgb(0xda70d6).into(), rgb(0x179fff).into()], rgb(0xf75464).into())
    } else {
        ([rgb(0x0431fa).into(), rgb(0x319331).into(), rgb(0x7b3814).into()], rgb(0xdb3b4b).into())
    }
}

/// The brackets of one open file and the colors laid over them.
#[derive(Default)]
pub(super) struct DocBrackets {
    pub scan: Rc<Brackets>,
    colors: Option<TextDecorationCollection>,
}

impl Workspace {
    /// Find the brackets of the file at `ix` again and color them.
    pub(super) fn refresh_brackets(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let scan = if self.app.bracket_colors {
            Brackets::scan(&doc.editor.read(cx).text().to_string(), doc.language)
        } else {
            Brackets::default()
        };
        let (levels, unmatched) = colors(cx.theme().is_dark());
        let decorations: Vec<TextDecoration> = scan
            .all
            .iter()
            .map(|bracket| {
                let color = bracket.depth.map_or(unmatched, |depth| levels[depth % levels.len()]);
                TextDecoration::new(
                    bracket.offset..bracket.offset + 1,
                    HighlightStyle {
                        color: Some(color),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let editor = doc.editor.clone();
        let doc = &mut self.docs[ix];
        doc.brackets.scan = Rc::new(scan);
        match &doc.brackets.colors {
            Some(collection) => collection.set(decorations, cx),
            None if decorations.is_empty() => {}
            None => {
                let collection = editor.update(cx, |editor, cx| editor.create_decorations_collection(decorations, cx));
                self.docs[ix].brackets.colors = Some(collection);
            }
        }
    }

    /// Color the brackets of every open file again, after the theme or the setting changed.
    pub(super) fn refresh_all_brackets(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.docs.len() {
            self.refresh_brackets(ix, cx);
        }
    }

    /// A layer over the editor of the file at `ix` that frames the bracket at
    /// the caret and its partner.
    pub(super) fn render_bracket_match(&self, ix: usize, cx: &App) -> Option<AnyElement> {
        if !self.app.bracket_colors {
            return None;
        }
        let doc = self.docs.get(ix)?;
        let scan = doc.brackets.scan.clone();
        if scan.all.is_empty() {
            return None;
        }
        let editor = doc.editor.clone();
        let theme = cx.theme();
        let fill = theme.foreground.opacity(if theme.is_dark() { 0.14 } else { 0.1 });
        let frame = theme.foreground.opacity(0.45);
        Some(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let state = editor.read(cx);
                    // Only the caret, not a selection, asks for its pair.
                    let selected = state.selected_range();
                    if !selected.is_empty() {
                        return;
                    }
                    let Some((open, close)) = scan.pair_at(state.cursor()) else {
                        return;
                    };
                    let text = state.text();
                    let is_bracket = |at: usize| text.char_at(at).is_some_and(|ch| "()[]{}".contains(ch));
                    if !is_bracket(open) || !is_bracket(close) {
                        return;
                    }
                    let visible = state.visible_row_range().unwrap_or_default();
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for at in [open, close] {
                            if !visible.contains(&text.offset_to_point(at).row) {
                                continue;
                            }
                            let Some(area) = state.range_to_bounds(&(at..at + 1)) else {
                                continue;
                            };
                            window.paint_quad(
                                quad(area, px(2.), fill, px(1.), frame, BorderStyle::Solid),
                            );
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
            .into_any_element(),
        )
    }
}
