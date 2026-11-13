//! The tab bar above the editor: switching, closing, dragging to reorder,
//! and the right-click menu.

use std::path::{Path, PathBuf};

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SidebarTab, Workspace, relative_path};
use crate::ui::{UI_TEXT, file_icon, icon};

const TAB_HEIGHT: f32 = 36.;

/// A tab being dragged, and what is drawn under the pointer meanwhile.
#[derive(Clone)]
pub(super) struct DraggedTab {
    /// Position in the tab bar.
    pub slot: usize,
    pub title: SharedString,
    pub diff: bool,
}

/// What sits at one position of the tab bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TabItem {
    Doc(usize),
    Diff,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .h(px(28.))
            .px_3()
            .gap_1p5()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.primary)
            .shadow_md()
            .text_size(UI_TEXT)
            .text_color(theme.foreground)
            .child(if self.diff {
                icon("git-compare").small().text_color(theme.primary)
            } else {
                file_icon(&self.title, cx).small()
            })
            .child(self.title.clone())
    }
}

/// Where a dragged tab would land: a bar before or after the tab it is over.
fn drop_marker(style: StyleRefinement, from: usize, to: usize, accent: Hsla) -> StyleRefinement {
    match from {
        from if from < to => style.border_r_2().border_color(accent),
        from if from > to => style.border_l_2().border_color(accent),
        _ => style,
    }
}

impl Workspace {
    /// The tabs in the order they are shown.
    pub(super) fn tab_order(&self) -> Vec<TabItem> {
        let mut order: Vec<TabItem> = (0..self.docs.len()).map(TabItem::Doc).collect();
        if self.diff.is_some() {
            order.insert(self.diff_slot.min(self.docs.len()), TabItem::Diff);
        }
        order
    }

    /// Move the tab at position `from` into the place of the tab at `to`.
    pub(super) fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let mut order = self.tab_order();
        if from == to || from >= order.len() || to >= order.len() {
            return;
        }
        let item = order.remove(from);
        order.insert(to, item);
        let active = self.active.map(|ix| self.docs[ix].path.clone());
        let mut old: Vec<Option<super::Doc>> =
            std::mem::take(&mut self.docs).into_iter().map(Some).collect();
        for item in order {
            match item {
                TabItem::Doc(ix) => self.docs.push(old[ix].take().expect("each file once")),
                TabItem::Diff => self.diff_slot = self.docs.len(),
            }
        }
        self.active = active.and_then(|path| self.docs.iter().position(|doc| doc.path == path));
        cx.notify();
    }

    fn doc_index(&self, path: &Path) -> Option<usize> {
        self.docs.iter().position(|doc| doc.path == path)
    }

    pub(super) fn close_path(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.doc_index(path) {
            self.close(ix, window, cx);
        }
    }

    /// Close every tab whose file `keep` does not accept; the diff tab goes too.
    fn close_where(
        &mut self,
        keep: impl Fn(usize, &Path) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let doomed: Vec<PathBuf> = self
            .docs
            .iter()
            .enumerate()
            .filter(|(ix, doc)| !keep(*ix, &doc.path))
            .map(|(_, doc)| doc.path.clone())
            .collect();
        for path in doomed {
            self.close_path(&path, window, cx);
        }
        if self.diff.is_some() {
            self.close_diff(window, cx);
        }
    }

    pub(super) fn close_others(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.close_where(|_, other| other == path, window, cx);
        if let Some(ix) = self.doc_index(path) {
            self.activate(ix, window, cx);
        }
    }

    pub(super) fn close_to_the_right(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(at) = self.doc_index(path) else {
            return;
        };
        let doomed: Vec<PathBuf> = self.docs[at + 1..].iter().map(|doc| doc.path.clone()).collect();
        let diff_after = self.diff.is_some() && self.diff_slot > at;
        for path in doomed {
            self.close_path(&path, window, cx);
        }
        if diff_after {
            self.close_diff(window, cx);
        }
    }

    pub(super) fn close_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_where(|_, _| false, window, cx);
    }

    /// Show `path` in the file tree.
    pub(super) fn reveal(&mut self, path: &Path, cx: &mut Context<Self>) {
        self.sidebar_visible = true;
        self.sidebar_tab = SidebarTab::Files;
        self.tree.update(cx, |tree, cx| tree.select(path, cx));
        cx.notify();
    }

    fn tab_menu(
        menu: PopupMenu,
        workspace: WeakEntity<Self>,
        path: PathBuf,
        relative: String,
        has_right: bool,
        has_others: bool,
    ) -> PopupMenu {
        let act = |label: &'static str,
                   enabled: bool,
                   run: fn(&mut Self, &Path, &mut Window, &mut Context<Self>)| {
            let workspace = workspace.clone();
            let path = path.clone();
            PopupMenuItem::new(label)
                .disabled(!enabled)
                .on_click(move |_, window, cx| {
                    let path = path.clone();
                    workspace
                        .update(cx, |this, cx| run(this, &path, window, cx))
                        .ok();
                })
        };
        let absolute = path.to_string_lossy().into_owned();
        menu.item(act("关闭", true, |this, path, window, cx| {
            this.close_path(path, window, cx)
        }))
        .item(act("关闭其他标签", has_others, Self::close_others))
        .item(act("关闭右侧标签", has_right, Self::close_to_the_right))
        .item(act("关闭所有标签", true, |this, _, window, cx| {
            this.close_all(window, cx)
        }))
        .separator()
        .item(PopupMenuItem::new("复制路径").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(absolute.clone()))
        }))
        .item(PopupMenuItem::new("复制相对路径").on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(relative.clone()))
        }))
        .item(act("在文件树中显示", true, |this, path, _, cx| {
            this.reveal(path, cx)
        }))
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let accent = theme.primary;
        let workspace = cx.entity().downgrade();
        let order = self.tab_order();
        let count = order.len();

        let tab = |id: ElementId, group: SharedString, active: bool| {
            h_flex()
                .id(id)
                .group(group)
                .relative()
                .h_full()
                .flex_shrink_0()
                .pl_3()
                .pr_1p5()
                .gap_1p5()
                .text_size(UI_TEXT)
                .text_color(if active {
                    theme.tab_active_foreground
                } else {
                    theme.tab_foreground
                })
                .when(!active, |tab| tab.hover(|tab| tab.bg(theme.list_hover)))
                // The active tab is underlined in the accent color.
                .when(active, |tab| {
                    tab.child(
                        div()
                            .absolute()
                            .left_0()
                            .right_0()
                            .bottom_0()
                            .h(px(2.))
                            .bg(accent),
                    )
                })
        };
        // The close button; a dot stands in for it while the file has unsaved edits.
        let close_button = |id: ElementId, group: SharedString, active: bool, dirty: bool| {
            let x_shown = active && !dirty;
            div()
                .id(id)
                .relative()
                .size(px(18.))
                .flex_shrink_0()
                .rounded(px(4.))
                .hover(|this| this.bg(theme.secondary_hover))
                .when(dirty, |this| {
                    this.child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .group_hover(group.clone(), |this| this.opacity(0.))
                            .child(div().size(px(7.)).rounded_full().bg(theme.foreground)),
                    )
                })
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(if x_shown { 1. } else { 0. })
                        .group_hover(group, |this| this.opacity(1.))
                        .child(icon("x").xsmall()),
                )
        };

        h_flex()
            .id("tabs")
            .h(px(TAB_HEIGHT))
            .w_full()
            .flex_shrink_0()
            .overflow_x_scroll()
            .children(order.iter().enumerate().map(|(slot, item)| {
                let element = match *item {
                    TabItem::Doc(ix) => {
                        let doc = &self.docs[ix];
                        let active = self.active == Some(ix) && !self.diff_active;
                        let group: SharedString = format!("tab-{ix}").into();
                        let path = doc.path.clone();
                        let menu_path = doc.path.clone();
                        let relative = relative_path(&self.root, &doc.path);
                        let workspace = workspace.clone();
                        tab(("tab", ix).into(), group.clone(), active)
                            .child(file_icon(&doc.title, cx).small())
                            .child(doc.title.clone())
                            .child(
                                close_button(("tab-close", ix).into(), group, active, doc.dirty)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_path(&path, window, cx);
                                    })),
                            )
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)),
                            )
                            .on_drag(
                                DraggedTab {
                                    slot,
                                    title: doc.title.clone(),
                                    diff: false,
                                },
                                |tab, _, _, cx| cx.new(|_| tab.clone()),
                            )
                            .drag_over::<DraggedTab>(move |style, dragged, _, _| {
                                drop_marker(style, dragged.slot, slot, accent)
                            })
                            .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                                this.move_tab(dragged.slot, slot, cx)
                            }))
                            .context_menu(move |menu, _, _| {
                                Self::tab_menu(
                                    menu,
                                    workspace.clone(),
                                    menu_path.clone(),
                                    relative.clone(),
                                    slot + 1 < count,
                                    count > 1,
                                )
                            })
                            .into_any_element()
                    }
                    TabItem::Diff => {
                        let diff = self.diff.as_ref().expect("the order lists the diff tab");
                        let workspace = workspace.clone();
                        tab("diff-tab".into(), "tab-diff".into(), self.diff_active)
                            .child(icon("git-compare").small().text_color(theme.primary))
                            .child(diff.title.clone())
                            .child(
                                close_button(
                                    "diff-tab-close".into(),
                                    "tab-diff".into(),
                                    self.diff_active,
                                    false,
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.close_diff(window, cx);
                                })),
                            )
                            .on_click(cx.listener(|this, _, window, cx| this.show_diff(window, cx)))
                            .on_drag(
                                DraggedTab {
                                    slot,
                                    title: diff.title.clone(),
                                    diff: true,
                                },
                                |tab, _, _, cx| cx.new(|_| tab.clone()),
                            )
                            .drag_over::<DraggedTab>(move |style, dragged, _, _| {
                                drop_marker(style, dragged.slot, slot, accent)
                            })
                            .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                                this.move_tab(dragged.slot, slot, cx)
                            }))
                            .context_menu(move |menu, _, _| {
                                let close = workspace.clone();
                                let close_all = workspace.clone();
                                menu.item(PopupMenuItem::new("关闭").on_click(move |_, window, cx| {
                                    close
                                        .update(cx, |this, cx| this.close_diff(window, cx))
                                        .ok();
                                }))
                                .item(PopupMenuItem::new("关闭所有标签").on_click(
                                    move |_, window, cx| {
                                        close_all
                                            .update(cx, |this, cx| this.close_all(window, cx))
                                            .ok();
                                    },
                                ))
                            })
                            .into_any_element()
                    }
                };
                // The slot lets UI tests find the tab; outside tests it is a plain box.
                div()
                    .id(("tab-slot", slot))
                    .h_full()
                    .flex_shrink_0()
                    .child(element)
                    .test_support()
            }))
            // Dropping past the last tab moves a tab to the end.
            .child(
                div()
                    .id("tabs-end")
                    .h_full()
                    .flex_1()
                    .min_w(px(40.))
                    .drag_over::<DraggedTab>(move |style, _, _, _| {
                        style.border_l_2().border_color(accent)
                    })
                    .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| {
                        this.move_tab(dragged.slot, count.saturating_sub(1), cx)
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn moving_a_tab_lands_it_in_the_target_slot() {
        // The same arithmetic `move_tab` does on the open files.
        let shuffle = |from: usize, to: usize| {
            let mut tabs = vec!['a', 'b', 'c', 'd'];
            let tab = tabs.remove(from);
            tabs.insert(to, tab);
            tabs.into_iter().collect::<String>()
        };
        assert_eq!(shuffle(0, 2), "bcad");
        assert_eq!(shuffle(3, 1), "adbc");
        assert_eq!(shuffle(1, 3), "acdb");
    }
}
