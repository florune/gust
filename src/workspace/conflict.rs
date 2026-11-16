//! Conflicts between unsaved edits and a newer version on disk. Neither side
//! is written over until the user keeps the edits, takes the disk's version,
//! or merges the two block by block in a compare view.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::*;

use super::Workspace;
use super::compare_view::{Entry, Source};
use crate::theme::palette;
use crate::ui::{UI_TEXT, icon};

impl Workspace {
    /// Keep the editor's text and write it over the disk's.
    pub(super) fn keep_mine(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.docs.get(ix).is_none_or(|doc| doc.conflict.is_none()) {
            return;
        }
        let path = self.docs[ix].path.clone();
        self.docs[ix].conflict = None;
        self.write_doc(ix, cx);
        self.close_disk_compare(&path, window, cx);
    }

    /// Drop the unsaved edits for what is on disk.
    pub(super) fn take_disk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        let Some(disk) = doc.conflict.take() else {
            return;
        };
        let path = doc.path.clone();
        let editor = doc.editor.clone();
        doc.saved = disk.clone();
        let before = editor.read(cx).text().to_string();
        super::replace_text(&editor, &before, &disk, window, cx);
        self.refresh_dirty(&editor, cx);
        self.close_disk_compare(&path, window, cx);
        cx.notify();
    }

    /// Compare the disk's version, on the left, with the unsaved edits.
    pub(super) fn compare_with_disk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let source = Source::Disk {
            file: doc.path.clone(),
            mine: doc.editor.read(cx).text().to_string(),
        };
        let entry = Entry {
            path: super::relative_path(&self.root, &doc.path),
            old_path: None,
            status: 'M',
        };
        self.show_compare(source, vec![entry], 0, window, cx);
    }

    /// The disk compare of `path` has nothing left to decide.
    fn close_disk_compare(&mut self, path: &std::path::Path, window: &mut Window, cx: &mut Context<Self>) {
        let showing = self
            .diff
            .as_ref()
            .is_some_and(|tab| matches!(&tab.source, Source::Disk { file, .. } if file == path));
        if showing {
            self.close_diff(window, cx);
        }
    }

    /// The edits of `path` changed: show them in its disk compare.
    pub(super) fn refresh_disk_compare(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        let Some(mine) = self
            .docs
            .iter()
            .find(|doc| doc.path == path)
            .map(|doc| doc.editor.read(cx).text().to_string())
        else {
            return;
        };
        let Some(tab) = &mut self.diff else {
            return;
        };
        if let Source::Disk { file, mine: shown } = &mut tab.source
            && file == path
        {
            *shown = mine;
            self.load_compare(true, cx);
        }
    }

    /// The bar above a file whose disk version changed under its edits.
    pub(super) fn render_conflict_bar(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let doc = self.docs.get(ix)?;
        doc.conflict.as_ref()?;
        let theme = cx.theme();
        let warning = palette(cx).orange;
        Some(
            h_flex()
                .id("conflict-bar")
                .w_full()
                .flex_shrink_0()
                .px_3()
                .py_1p5()
                .gap_2()
                .text_size(UI_TEXT)
                .bg(warning.opacity(0.14))
                .border_b_1()
                .border_color(warning.opacity(0.4))
                .child(icon("triangle-alert").small().text_color(warning))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.foreground)
                        .child("文件已在磁盘上被修改，与未保存的修改冲突。"),
                )
                .child(
                    Button::new("conflict-compare")
                        .small()
                        .primary()
                        .label("比较")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.compare_with_disk(ix, window, cx)
                        })),
                )
                .child(
                    Button::new("conflict-disk")
                        .small()
                        .label("使用磁盘版本")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.take_disk(ix, window, cx)
                        })),
                )
                .child(
                    Button::new("conflict-mine")
                        .small()
                        .label("保留我的修改")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.keep_mine(ix, window, cx)
                        })),
                )
                .into_any_element(),
        )
    }
}
