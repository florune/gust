//! The tab bar above the editor: switching, closing, dragging to reorder,
//! and the right-click menu.

use std::path::{Path, PathBuf};

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{SidebarTab, Workspace, relative_path};
use crate::ui::{UI_SMALL, UI_TEXT, file_icon, icon};

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

    #[allow(clippy::too_many_arguments)]
    fn tab_menu(
        menu: PopupMenu,
        workspace: WeakEntity<Self>,
        path: PathBuf,
        relative: String,
        has_right: bool,
        has_others: bool,
        worktrees: Vec<(String, String, PathBuf)>,
        checkout: PathBuf,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
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
        .separator()
        .item(act("向右拆分", true, |this, path, window, cx| {
            this.split_right(path, window, cx)
        }))
        .item(act("本地历史…", true, |this, path, window, cx| {
            this.show_local_history(path.to_path_buf(), window, cx)
        }))
        .when_some(
            {
                // Only the worktrees that have this file.
                let inside = path.strip_prefix(&checkout).ok().map(Path::to_path_buf);
                inside.and_then(|inside| {
                    let found: Vec<_> = worktrees
                        .into_iter()
                        .filter(|(_, _, root)| root.join(&inside).is_file())
                        .collect();
                    (!found.is_empty()).then_some(found)
                })
            },
            |menu, found| {
                let workspace = workspace.clone();
                let path = path.clone();
                menu.separator().submenu("与其他工作树比较", window, cx, move |menu, window, _| {
                    let mut menu = Self::long_menu(menu, window).min_w(px(240.)).max_w(px(520.));
                    for (name, branch, root) in &found {
                        let workspace = workspace.clone();
                        let (path, root, name) = (path.clone(), root.clone(), name.clone());
                        let (shown, branch) = (name.clone(), branch.clone());
                        let row = move |_: &mut Window, cx: &mut App| {
                            let theme = cx.theme();
                            h_flex()
                                .w_full()
                                .min_w_0()
                                .gap_2()
                                .child(div().flex_shrink_0().child(shown.clone()))
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(UI_SMALL)
                                        .text_color(theme.muted_foreground)
                                        .child(branch.clone()),
                                )
                        };
                        menu = menu.item(PopupMenuItem::element(row).on_click(move |_, window, cx| {
                            let (path, root, name) = (path.clone(), root.clone(), name.clone());
                            workspace
                                .update(cx, |this, cx| this.compare_with_worktree(&path, &root, name, window, cx))
                                .ok();
                        }));
                    }
                    menu
                })
            },
        )
    }

    /// When the tabs do not all fit: a button listing every one of them.
    pub(super) fn render_tab_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        use gpui_kit::component::menu::DropdownMenu as _;
        if self.tab_scroll.max_offset().x <= px(1.) {
            return None;
        }
        let workspace = cx.entity().downgrade();
        let items: Vec<(TabItem, SharedString, String, bool)> = self
            .tab_order()
            .into_iter()
            .filter_map(|item| match item {
                TabItem::Doc(ix) => {
                    let doc = self.docs.get(ix)?;
                    let folder = doc
                        .path
                        .parent()
                        .map(|dir| super::relative_path(&self.root, dir))
                        .unwrap_or_default();
                    Some((item, doc.title.clone(), folder, !self.diff_active && self.active == Some(ix)))
                }
                TabItem::Diff => {
                    let diff = self.diff.as_ref()?;
                    Some((item, diff.title.clone(), "对比".to_string(), self.diff_active))
                }
            })
            .collect();
        let count = items.len();
        Some(
            div()
                .flex_shrink_0()
                .px_0p5()
                .child(
                    Button::new("tab-list")
                        .ghost()
                        .xsmall()
                        .icon(icon("chevron-down"))
                        .tooltip(format!("全部 {count} 个标签"))
                        .dropdown_menu(move |menu, window, _| {
                            let mut menu = Self::long_menu(menu, window).min_w(px(260.)).max_w(px(560.));
                            for (item, title, folder, active) in &items {
                                let workspace = workspace.clone();
                                let item = *item;
                                let (title, folder) = (title.clone(), folder.clone());
                                // The name, and where it is, quieter.
                                let row = move |_: &mut Window, cx: &mut App| {
                                    let theme = cx.theme();
                                    h_flex()
                                        .w_full()
                                        .min_w_0()
                                        .gap_2()
                                        .child(crate::ui::file_icon(&title, cx).small())
                                        .child(div().flex_shrink_0().child(title.clone()))
                                        .child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .text_size(UI_SMALL)
                                                .text_color(theme.muted_foreground)
                                                .child(folder.clone()),
                                        )
                                };
                                menu = menu.item(PopupMenuItem::element(row).checked(*active).on_click(
                                    move |_, window, cx| {
                                        workspace
                                            .update(cx, |this, cx| match item {
                                                TabItem::Doc(ix) => this.select_tab(ix, window, cx),
                                                TabItem::Diff => this.show_diff(window, cx),
                                            })
                                            .ok();
                                    },
                                ));
                            }
                            menu
                        }),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        // The tab just made active comes into view, however far the bar is scrolled.
        let active_slot = self.tab_order().iter().position(|item| match item {
            TabItem::Doc(ix) => !self.diff_active && self.active == Some(*ix),
            TabItem::Diff => self.diff_active,
        });
        // A narrower bar may hide it again, so a change of width counts too.
        let width = self.tab_scroll.bounds().size.width;
        if active_slot != self.tab_in_view.get() || width != self.tab_bar_width.get() {
            // Before the bar has been laid out once, a request to scroll is
            // dropped: ask again on the next frame.
            if width > px(0.) {
                self.tab_in_view.set(active_slot);
                self.tab_bar_width.set(width);
                if let Some(slot) = active_slot {
                    self.tab_scroll.scroll_to_item(slot);
                }
            } else {
                cx.notify();
            }
        }
        let theme = cx.theme();
        let accent = theme.primary;
        let workspace = cx.entity().downgrade();
        let order = self.tab_order();
        let count = order.len();
        let split_path = self.split.as_ref().map(|split| split.path.clone());
        let others = self.other_worktrees();
        let checkout = self.checkout_top();

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
            .track_scroll(&self.tab_scroll)
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
                            // Files of the same name say which folder they are in.
                            .when_some(
                                (self.docs.iter().filter(|other| other.title == doc.title).count() > 1)
                                    .then(|| doc.path.parent().and_then(|dir| dir.file_name()))
                                    .flatten()
                                    .map(|folder| folder.to_string_lossy().into_owned()),
                                |tab, folder| {
                                    tab.child(
                                        div()
                                            .text_size(UI_SMALL)
                                            .text_color(theme.muted_foreground)
                                            .child(folder),
                                    )
                                },
                            )
                            .when(split_path.as_ref() == Some(&doc.path), |tab| {
                                tab.child(icon("columns-2").xsmall().text_color(theme.muted_foreground))
                            })
                            .child(
                                close_button(("tab-close", ix).into(), group, active, doc.dirty)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.close_path(&path, window, cx);
                                    })),
                            )
                            .on_click(
                                cx.listener(move |this, _, window, cx| this.select_tab(ix, window, cx)),
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
                            .context_menu({
                                let others = others.clone();
                                let checkout = checkout.clone();
                                move |menu, window, cx| Self::tab_menu(
                                    menu,
                                    workspace.clone(),
                                    menu_path.clone(),
                                    relative.clone(),
                                    slot + 1 < count,
                                    count > 1,
                                    others.clone(),
                                    checkout.clone(),
                                    window,
                                    cx,
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
