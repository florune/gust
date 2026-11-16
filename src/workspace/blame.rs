//! Who last changed the line the caret is on, written faintly after its end,
//! the way GitLens does: author, how long ago, and the commit's subject.

use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, RopeExt as _};
use gpui_kit::*;

use super::Workspace;
use crate::history;

/// Room between the end of the line and the note.
const GAP: f32 = 4.;

impl Workspace {
    /// Read the blame of the file at `ix` again, after typing pauses unless `now`.
    pub(super) fn refresh_blame(&mut self, ix: usize, now: bool, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let Some((root, path)) = self.repo_path(&doc.path) else {
            self.docs[ix].blame = Rc::default();
            return;
        };
        let text = doc.editor.read(cx).text().to_string();
        let file = doc.path.clone();
        let task = cx.spawn(async move |this, cx| {
            if !now {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
            }
            let blame = cx
                .background_executor()
                .spawn(async move { history::blame(&root, &path, &text) })
                .await;
            this.update(cx, |this, cx| {
                if let Some(doc) = this.docs.iter_mut().find(|doc| doc.path == file) {
                    doc.blame = Rc::new(blame.unwrap_or_default());
                    cx.notify();
                }
            })
            .ok();
        });
        self.docs[ix].blame_task = task;
    }

    /// The note for the caret's line of the file at `ix`, if git knows it.
    pub(super) fn blame_note(&self, ix: usize, cx: &App) -> Option<SharedString> {
        let doc = self.docs.get(ix)?;
        let row = doc.editor.read(cx).cursor_position().line as usize;
        let entry = doc.blame.get(row)?.as_ref()?;
        Some(if entry.uncommitted() {
            "你 · 未提交的修改".into()
        } else {
            format!(
                "{}, {} · {}",
                entry.author,
                super::log::when(entry.time),
                entry.summary
            )
            .into()
        })
    }

    /// A layer over the editor of the file at `ix` that draws the note.
    pub(super) fn render_blame_layer(&self, ix: usize, cx: &App) -> Option<AnyElement> {
        let note = self.blame_note(ix, cx)?;
        let editor = self.docs.get(ix)?.editor.clone();
        let theme = cx.theme();
        let color = theme.muted_foreground.opacity(0.75);
        let font = Font {
            family: theme.mono_font_family.clone(),
            style: FontStyle::Italic,
            ..Default::default()
        };
        let font_size = theme.mono_font_size;
        Some(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let state = editor.read(cx);
                    let row = state.cursor_position().line as usize;
                    let end = state.text().line_end_offset(row);
                    let Some(at) = state.range_to_bounds(&(end..end)) else {
                        return;
                    };
                    let line_height = at.size.height;
                    let text_size = font_size.min(line_height * 0.8);
                    let run = TextRun {
                        len: note.len(),
                        font: font.clone(),
                        color,
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let line = window
                        .text_system()
                        .shape_line(note.clone(), text_size, &[run], None);
                    let origin = point(at.origin.x + text_size * GAP, at.origin.y);
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
                    });
                },
            )
            .absolute()
            .inset_0()
            .into_any_element(),
        )
    }
}
