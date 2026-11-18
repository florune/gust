//! The window's frame: title bar, activity bar, side panel, editor area,
//! bottom panel, and status bar.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, TitleBar, h_flex, h_resizable, resizable_panel, v_flex,
    v_resizable,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::palette::Mode;
use super::{
    NewTerminal, OpenFolder, ShowGitLog, SidebarTab, ToggleTerminal, ToggleTheme, Workspace,
    relative_path,
};
use crate::lang;
use crate::ui::{UI_SMALL, UI_TEXT, app_logo, icon};

const ACTIVITY_WIDTH: f32 = 44.;
const PANEL_HEADER: f32 = 36.;

/// One of the bottom panel's tools.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum BottomTab {
    Terminal,
    Log,
    /// The console of the program being built and run.
    Run,
}

impl Workspace {
    /// Show the side panel's `tab`; asking for the one already shown hides the panel.
    /// A menu that may hold many items: it scrolls, and stays inside the window.
    pub(super) fn long_menu(
        menu: gpui_kit::component::menu::PopupMenu,
        window: &Window,
    ) -> gpui_kit::component::menu::PopupMenu {
        let room = (window.viewport_size().height - px(120.)).max(px(200.));
        menu.scrollable(true).max_h(room)
    }

    pub(super) fn toggle_sidebar_tab(&mut self, tab: SidebarTab, cx: &mut Context<Self>) {
        if self.sidebar_visible && self.sidebar_tab == tab {
            self.sidebar_visible = false;
        } else {
            self.sidebar_visible = true;
            self.sidebar_tab = tab;
        }
        self.watch_worktrees(cx);
        cx.notify();
    }

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let branch = self.render_branch(cx);
        let project = self.render_project_menu(cx);
        let app_menu = self.render_app_menu(cx);
        let worktree = self.render_worktree_marker(cx);
        let theme = cx.theme();
        // Presses on the controls must not start dragging the window.
        let no_drag = |_: &MouseDownEvent, _: &mut Window, cx: &mut App| cx.stop_propagation();
        // The window's own buttons come after this row. Laid over the space
        // beside them rather than sized by its contents, the row never pushes
        // them out of the window: when it is narrow, its parts shrink and clip.
        TitleBar::new().child(
            h_flex()
                .absolute()
                .top_0()
                .bottom_0()
                .left_0()
                .right_0()
                .gap_1()
                .overflow_hidden()
                .text_size(UI_TEXT)
                .child(
                    div()
                        .on_mouse_down(MouseButton::Left, no_drag)
                        .child(app_menu),
                )
                .child(
                    div()
                        .ml_1()
                        .flex_shrink_0()
                        .on_mouse_down(MouseButton::Left, no_drag)
                        .child(project),
                )
                .children(worktree.map(|worktree| {
                    div()
                        .min_w_0()
                        .flex_shrink(1.)
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, no_drag)
                        .child(worktree)
                }))
                .children(branch.map(|branch| {
                    div()
                        .min_w_0()
                        .flex_shrink(1.)
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, no_drag)
                        .child(branch)
                }))
                .child(div().flex_1().min_w_2())
                .child(
                    h_flex()
                        .id("find-file")
                        .flex_shrink(1.)
                        .min_w(px(150.))
                        .w(px(380.))
                        .overflow_hidden()
                        .h(px(26.))
                        .px_2()
                        .gap_2()
                        .rounded(theme.radius)
                        .bg(theme.background.opacity(0.6))
                        .border_1()
                        .border_color(theme.border)
                        .text_size(UI_SMALL)
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(|this| this.border_color(theme.ring).text_color(theme.foreground))
                        .child(icon("search").xsmall())
                        .child(div().min_w_0().truncate().child(format!("搜索 {}", self.project_name())))
                        .child(div().flex_1())
                        .child(crate::ui::key_chip(self.shortcut("find_file"), cx))
                        .on_mouse_down(MouseButton::Left, no_drag)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_palette(Mode::Files, window, cx)
                        })),
                )
                .child(div().flex_1().min_w_2())
                .child(
                    div()
                        .mr_2()
                        .flex_shrink_0()
                        .on_mouse_down(MouseButton::Left, no_drag)
                        .child(self.render_run_toolbar(cx)),
                )
                // Room for the window buttons, so the search box sits mid-window;
                // the first thing given up when space runs short.
                .child(div().w(px(120.)).flex_shrink(20.).min_w_0()),
        )
    }

    /// The project name, which opens a menu of the projects opened before
    /// and a way to open another folder.
    fn render_project_menu(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};

        let workspace = cx.entity().downgrade();
        let current = self.root.to_string_lossy().into_owned();
        let recent = self.recent.clone();
        let name = self.project_name();
        Button::new("project")
            .ghost()
            .small()
            .child(
                h_flex()
                    .gap_1p5()
                    .child(crate::ui::project_avatar(&name, px(18.)))
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone())),
            )
            .dropdown_caret(true)
            .dropdown_menu(move |menu, window, _| {
                let mut menu = Self::long_menu(menu, window);
                let others: Vec<_> = recent
                    .iter()
                    .filter(|project| !project.path.eq_ignore_ascii_case(&current))
                    .cloned()
                    .collect();
                if !others.is_empty() {
                    menu = menu.label("最近打开");
                }
                for project in others {
                    let open = workspace.clone();
                    let path = project.path.clone();
                    let name = project.name();
                    let shown = project.path.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            h_flex()
                                .py_0p5()
                                .gap_2()
                                .child(crate::ui::project_avatar(&name, px(24.)))
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .child(div().text_size(UI_TEXT).child(name.clone()))
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(UI_SMALL)
                                                .text_color(cx.theme().muted_foreground)
                                                .child(shown.clone()),
                                        ),
                                )
                        })
                        .on_click(move |_, window, cx| {
                            let path = std::path::PathBuf::from(&path);
                            open.update(cx, |this, cx| this.choose_project(path, window, cx))
                                .ok();
                        }),
                    );
                }
                let open = workspace.clone();
                menu.separator().item(
                    PopupMenuItem::new("打开文件夹…")
                        .icon(icon("folder-open"))
                        .on_click(move |_, window, cx| {
                            open.update(cx, |this, cx| {
                                this.on_open_folder(&OpenFolder, window, cx)
                            })
                            .ok();
                        }),
                )
            })
    }

    fn render_activity_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let changes = self.git.as_ref().map_or(0, |status| status.changes.len());
        let button = |id: &'static str, name: &'static str, tip: SharedString, active: bool| {
            div()
                .id(id)
                .relative()
                .size(px(32.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .when(active, |this| this.bg(theme.secondary_hover))
                .when(!active, |this| {
                    this.hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
                })
                .child(icon(name).size(px(18.)))
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        };
        let files = self.sidebar_visible && self.sidebar_tab == SidebarTab::Files;
        let worktrees = self.sidebar_visible && self.sidebar_tab == SidebarTab::Worktrees;
        let commit = self.sidebar_visible && self.sidebar_tab == SidebarTab::Changes;
        v_flex()
            .w(px(ACTIVITY_WIDTH))
            .h_full()
            .flex_shrink_0()
            .items_center()
            .py_1p5()
            .gap_1p5()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(button("activity-files", "folder", self.tip("项目", "sidebar"), files).on_click(
                cx.listener(|this, _, _, cx| this.toggle_sidebar_tab(SidebarTab::Files, cx)),
            ))
            .child(
                button("activity-commit", "git-commit-horizontal", self.tip("提交", "commit"), commit)
                    .when(changes > 0, |this| {
                        this.child(
                            div()
                                .absolute()
                                .top(px(2.))
                                .right(px(1.))
                                .min_w(px(14.))
                                .h(px(14.))
                                .px(px(3.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(theme.primary)
                                .text_color(theme.primary_foreground)
                                .text_size(px(9.))
                                .child(if changes > 99 {
                                    "99+".to_string()
                                } else {
                                    changes.to_string()
                                }),
                        )
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_sidebar_tab(SidebarTab::Changes, cx)
                    })),
            )
            .child(
                button("activity-worktrees", "git-fork", self.tip("工作树", "worktrees"), worktrees)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar_tab(SidebarTab::Worktrees, cx))),
            )
            .child(div().flex_1())
            .child(
                button(
                    "activity-log",
                    "git-graph",
                    self.tip("Git 日志", "git_log"),
                    self.terminal_visible && self.bottom_tab == BottomTab::Log,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.on_show_git_log(&ShowGitLog, window, cx)
                })),
            )
            .child(
                button(
                    "activity-terminal",
                    "square-terminal",
                    self.tip("终端", "terminal"),
                    self.terminal_visible && self.bottom_tab == BottomTab::Terminal,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.on_toggle_terminal(&ToggleTerminal, window, cx)
                })),
            )
            .child(
                button(
                    "activity-theme",
                    if theme.is_dark() { "sun" } else { "moon" },
                    self.tip("切换浅色/深色", "toggle_theme"),
                    false,
                )
                .on_click(cx.listener(|this, _, window, cx| {
                    this.on_toggle_theme(&ToggleTheme, window, cx)
                })),
            )
    }

    /// A small icon button for panel headers.
    pub(super) fn header_button(
        id: impl Into<ElementId>,
        name: &'static str,
        tip: &'static str,
        cx: &App,
    ) -> Stateful<Div> {
        let theme = cx.theme();
        div()
            .id(id)
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(5.))
            .text_color(theme.muted_foreground)
            .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
            .child(icon(name).small())
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let content = match self.sidebar_tab {
            SidebarTab::Files => self.tree.clone().into_any_element(),
            SidebarTab::Changes => self.render_changes(cx),
            SidebarTab::Worktrees => self.render_worktrees(cx),
        };
        let theme = cx.theme();
        let title = match self.sidebar_tab {
            SidebarTab::Files => "项目",
            SidebarTab::Changes => "提交",
            SidebarTab::Worktrees => "工作树",
        };
        let actions = match self.sidebar_tab {
            SidebarTab::Files => h_flex()
                .gap_0p5()
                .child(
                    Self::header_button("tree-new-file", "file-plus", "新建文件", cx).on_click(
                        cx.listener(|this, _, window, cx| {
                            let dir = this.new_file_dir(cx);
                            this.open_file_prompt(
                                super::file_ops::FileAction::New { dir, folder: false },
                                window,
                                cx,
                            )
                        }),
                    ),
                )
                .child(
                    Self::header_button("tree-locate", "locate-fixed", "定位当前文件", cx).on_click(
                        cx.listener(|this, _, _, cx| {
                            if let Some(path) =
                                this.active.and_then(|ix| this.docs.get(ix)).map(|d| d.path.clone())
                            {
                                this.reveal(&path, cx);
                            }
                        }),
                    ),
                )
                .child(
                    Self::header_button(
                        "tree-hidden",
                        if self.tree.read(cx).shows_hidden() { "eye" } else { "eye-off" },
                        if self.tree.read(cx).shows_hidden() {
                            "不显示已隐藏的文件"
                        } else {
                            "显示已隐藏的文件"
                        },
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.tree.update(cx, |tree, cx| tree.toggle_show_hidden(cx))
                    })),
                )
                .child(
                    Self::header_button("tree-collapse", "chevrons-down-up", "全部折叠", cx)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.tree.update(cx, |tree, cx| tree.collapse_all(cx))
                        })),
                ),
            SidebarTab::Changes => h_flex().gap_0p5().child(
                Self::header_button("changes-refresh", "refresh-cw", "刷新", cx)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_git(cx))),
            ),
            SidebarTab::Worktrees => h_flex().gap_0p5().child(
                Self::header_button("worktree-new", "plus", "新建工作树", cx)
                    .on_click(cx.listener(|this, _, window, cx| this.open_new_worktree(window, cx))),
            ),
        };
        v_flex()
            .size_full()
            .bg(theme.sidebar)
            .child(
                h_flex()
                    .h(px(PANEL_HEADER))
                    .flex_shrink_0()
                    .pl_3()
                    .pr_1p5()
                    .text_size(UI_TEXT)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title)
                    .child(div().flex_1())
                    .child(actions),
            )
            .child(div().flex_1().min_h_0().child(content))
    }

    fn render_empty_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = |label: &'static str, id: &str| {
            let keys = self.shortcut(id);
            h_flex()
                .w(px(300.))
                .h(px(28.))
                .text_size(UI_TEXT)
                .child(div().flex_1().text_color(theme.muted_foreground).child(label))
                .child(
                    div()
                        .px_1p5()
                        .rounded(px(4.))
                        .bg(theme.secondary)
                        .text_size(UI_SMALL)
                        .text_color(theme.foreground)
                        .child(keys),
                )
        };
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_1()
            .bg(theme.background)
            .child(
                div().mb_6().opacity(0.85).child(app_logo(px(64.))),
            )
            .child(row("搜索文件", "find_file"))
            .child(row("全局搜索", "find_text"))
            .child(row("终端", "terminal"))
            .child(row("提交", "commit"))
            .child(row("Git 日志", "git_log"))
            .child(row("打开文件夹", "open_folder"))
            .into_any_element()
    }

    fn render_center(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let diff = self
            .diff
            .as_ref()
            .filter(|_| self.diff_active)
            .map(|tab| self.render_diff(tab, cx));
        let body = match self.active.and_then(|ix| self.docs.get(ix)) {
            _ if diff.is_some() => diff.unwrap(),
            Some(_) => {
                let ix = self.active.unwrap();
                let bar = self.render_conflict_bar(ix, cx);
                let left = v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(bar)
                    .child(self.render_doc(ix, cx));
                h_flex()
                    .size_full()
                    .child(left)
                    .children(self.render_split(cx))
                    .into_any_element()
            }
            None => self.render_empty_editor(cx),
        };
        let has_tabs = !self.docs.is_empty() || self.diff.is_some();
        let switch = self.render_view_switch(cx);
        let split_button = self.render_split_button(cx);
        // The view switch sits at the end of the tab bar, clear of the
        // editor's own search bar.
        let tabs = has_tabs.then(|| {
            h_flex()
                .w_full()
                .flex_shrink_0()
                .bg(cx.theme().background)
                .border_b_1()
                .border_color(cx.theme().border)
                .child(div().flex_1().min_w_0().child(self.render_tabs(cx)))
                .children(self.render_tab_list(cx))
                .children(switch)
                .children(split_button)
        });
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .children(tabs)
            .child(body)
    }

    fn render_bottom_panel(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let log_toolbar = (self.bottom_tab == BottomTab::Log).then(|| self.render_log_toolbar(cx));
        let log_body = (self.bottom_tab == BottomTab::Log).then(|| self.render_log(cx));
        let terminal = self.bottom_tab == BottomTab::Terminal;
        let running = self.bottom_tab == BottomTab::Run;
        let run_status = running.then(|| self.render_run_status(cx));
        let has_console = self.run.console.is_some();
        let theme = cx.theme();
        let tool = |id: &'static str, label: &'static str, active: bool| {
            div()
                .id(id)
                .relative()
                .h_full()
                .px_2()
                .flex()
                .items_center()
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.muted_foreground
                })
                .when(active, |this| this.font_weight(FontWeight::SEMIBOLD))
                .hover(|this| this.text_color(theme.foreground))
                .child(label)
                .when(active, |this| {
                    this.child(
                        div()
                            .absolute()
                            .left_2()
                            .right_2()
                            .bottom_0()
                            .h(px(2.))
                            .bg(theme.primary),
                    )
                })
        };
        let header = h_flex()
            .id("bottom-header")
            .h(px(PANEL_HEADER))
            .w_full()
            .flex_shrink_0()
            .pl_3()
            .pr_1p5()
            .gap_1()
            .bg(theme.sidebar)
            .border_b_1()
            .border_color(theme.border)
            .text_size(UI_TEXT)
            .child(
                tool("bottom-terminal", "终端", terminal).on_click(cx.listener(
                    |this, _, window, cx| {
                        if this.terminals.is_empty() {
                            this.new_terminal(window, cx);
                        } else {
                            this.show_terminal(this.active_terminal, window, cx);
                        }
                    },
                )),
            )
            .child(
                tool("bottom-log", "Git 日志", self.bottom_tab == BottomTab::Log)
                    .when(!has_console, |tab| tab.mr_2())
                    .on_click(cx.listener(|this, _, window, cx| this.show_log(window, cx))),
            )
            .when(has_console, |header| {
                header.child(
                    tool("bottom-run", "运行", running)
                        .mr_2()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.terminal_visible = true;
                            this.bottom_tab = BottomTab::Run;
                            cx.notify();
                        })),
                )
            })
            .when(running, |header| {
                header.child(
                    div()
                        .text_size(UI_SMALL)
                        .text_color(theme.muted_foreground)
                        .child(self.run.title.clone()),
                )
            })
            .when(terminal, |header| {
                header.children(self.terminals.iter().enumerate().map(|(ix, tab)| {
                let active = self.active_terminal == ix;
                let group: SharedString = format!("terminal-{ix}").into();
                h_flex()
                    .id(("terminal-tab", ix))
                    .group(group.clone())
                    .h(px(26.))
                    .pl_2()
                    .pr_1()
                    .gap_1()
                    .rounded(px(5.))
                    .text_color(theme.muted_foreground)
                    .when(active, |tab| {
                        tab.bg(theme.secondary_hover).text_color(theme.foreground)
                    })
                    .when(!active, |tab| tab.hover(|tab| tab.bg(theme.list_hover)))
                    .child(icon("terminal").xsmall())
                    .child(tab.name.clone())
                    .child(
                        div()
                            .id(("terminal-close", ix))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(4.))
                            .opacity(if active { 1. } else { 0. })
                            .group_hover(group, |this| this.opacity(1.))
                            .hover(|this| this.bg(theme.secondary_active))
                            .child(icon("x").xsmall())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close_terminal(ix, window, cx);
                            })),
                    )
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.show_terminal(ix, window, cx)),
                    )
                }))
                .child(
                    Self::header_button("terminal-new", "plus", "新建终端 (Ctrl+Shift+`)", cx)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_new_terminal(&NewTerminal, window, cx)
                        })),
                )
            })
            .child(div().flex_1())
            .children(log_toolbar)
            .children(run_status)
            .child(
                Self::header_button("bottom-hide", "minus", "隐藏", cx).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.terminal_visible = false;
                        this.focus_editor(window, cx);
                        cx.notify();
                    },
                )),
            );
        let body = match self.bottom_tab {
            BottomTab::Terminal => self
                .terminals
                .get(self.active_terminal)
                .map(|tab| tab.view.clone().into_any_element()),
            BottomTab::Log => log_body,
            BottomTab::Run => self
                .run
                .console
                .clone()
                .map(|console| console.into_any_element()),
        };
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(header)
            .children(body.map(|body| div().flex_1().min_h_0().child(body)))
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let doc = self
            .active
            .and_then(|ix| self.docs.get(ix))
            .filter(|_| !self.diff_active);
        let position = doc.map(|doc| doc.editor.read(cx).cursor_position());
        let symbol = doc.and_then(|_| self.current_symbol(cx));
        let (errors, warnings) = self.problems.counts();
        let item = |text: String| {
            div()
                .px_1p5()
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
                .child(text)
        };
        h_flex()
            .h(px(26.))
            .w_full()
            .flex_shrink_0()
            .px_2()
            .gap_0p5()
            .text_size(UI_SMALL)
            .text_color(theme.muted_foreground)
            .bg(theme.status_bar)
            .border_t_1()
            .border_color(theme.status_bar_border)
            .when_some(self.info.clone(), |this, info| {
                this.child(
                    h_flex()
                        .gap_1()
                        .px_1p5()
                        .min_w_0()
                        .text_color(theme.success)
                        .child(icon("circle-check").xsmall())
                        .child(div().truncate().child(info)),
                )
            })
            .when_some(self.notice.clone(), |this, notice| {
                this.child(
                    h_flex()
                        .gap_1()
                        .px_1p5()
                        .min_w_0()
                        .text_color(theme.danger)
                        .child(icon("circle-alert").xsmall())
                        .child(div().truncate().child(notice)),
                )
            })
            .child(div().flex_1())
            .when(errors + warnings > 0, |this| {
                let colors = crate::theme::palette(cx);
                this.child(
                    h_flex()
                        .id("problems")
                        .h(px(20.))
                        .px_1p5()
                        .gap_2()
                        .rounded(px(4.))
                        .hover(|this| this.bg(theme.list_hover))
                        .when(errors > 0, |row| {
                            row.child(
                                h_flex()
                                    .gap_1()
                                    .text_color(colors.red)
                                    .child(icon("circle-x").xsmall())
                                    .child(errors.to_string()),
                            )
                        })
                        .when(warnings > 0, |row| {
                            row.child(
                                h_flex()
                                    .gap_1()
                                    .text_color(colors.yellow)
                                    .child(icon("triangle-alert").xsmall())
                                    .child(warnings.to_string()),
                            )
                        })
                        .tooltip(|window, cx| Tooltip::new("问题 (Alt+6)").build(window, cx))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_show_problems(&super::problems_view::ShowProblems, window, cx)
                        })),
                )
            })
            .when_some(doc, |this, doc| {
                this.child(item(relative_path(&self.root, &doc.path).replace('/', " › ")))
            })
            .when_some(symbol, |this, symbol| {
                this.child(
                    item(format!("› {symbol}"))
                        .id("current-symbol")
                        .text_color(theme.foreground)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.on_show_outline(&super::outline::ShowOutline, window, cx)
                        })),
                )
            })
            .when_some(position, |this, position| {
                this.child(item(format!(
                    "{}:{}",
                    position.line + 1,
                    position.character + 1
                )))
            })
            .when_some(doc, |this, doc| {
                let indent = if matches!(doc.language, "go" | "make") {
                    "Tab"
                } else {
                    "4 空格"
                };
                this.child(item(if doc.crlf { "CRLF" } else { "LF" }.to_string()))
                    .child(item("UTF-8".to_string()))
                    .child(item(indent.to_string()))
                    .child(item(lang::display_name(doc.language).to_string()))
            })
            .child(item(format!("{} MB", self.memory_mb)))
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_lenses(cx);
        self.update_window_title(window);
        let palette = self.render_palette(cx);
        let run_configs = self.render_run_configs(cx);
        let file_prompt = self.render_file_prompt(cx);
        let hunk = self.render_hunk(cx);
        let project_choice = self.render_project_choice(cx);
        let quick_doc = self.render_quick_doc(cx);
        let settings_page = self.render_settings_page(cx);
        let keymap_page = self.render_keymap_page(cx);
        let about = self.render_about(cx);
        let new_worktree = self.render_new_worktree(cx);
        let merge_worktree = self.render_merge_worktree(cx);
        let merge_dialog = self.render_merge_dialog(cx);
        let zoom = self.render_zoom_listener(cx);
        let autoscroll = self.render_autoscroll(cx);
        let sidebar = self.render_sidebar(cx);
        let center = self.render_center(cx);
        let bottom = self.render_bottom_panel(cx);
        let theme = cx.theme();

        v_flex()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_save))
            .on_action(cx.listener(Self::on_format))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_show_commit))
            .on_action(cx.listener(Self::on_toggle_theme))
            .on_action(cx.listener(Self::on_open_folder))
            .on_action(cx.listener(Self::on_toggle_terminal))
            .on_action(cx.listener(Self::on_new_terminal))
            .on_action(cx.listener(Self::on_show_git_log))
            .on_action(cx.listener(Self::on_find_usages))
            .on_action(cx.listener(Self::on_show_outline))
            .on_action(cx.listener(Self::on_split_right))
            .on_action(cx.listener(Self::on_navigate_back))
            .on_action(cx.listener(Self::on_extend_selection))
            .on_action(cx.listener(Self::on_toggle_comment))
            .on_action(cx.listener(Self::on_duplicate_line))
            .on_action(cx.listener(Self::on_delete_line))
            .on_action(cx.listener(Self::on_move_line_up))
            .on_action(cx.listener(Self::on_move_line_down))
            .on_action(cx.listener(Self::on_recent_files))
            .on_action(cx.listener(Self::on_next_problem))
            .on_action(cx.listener(Self::on_rollback_lines))
            .on_action(cx.listener(Self::on_open_here))
            .on_action(cx.listener(Self::on_quick_documentation))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_open_keymap))
            .on_action(cx.listener(Self::on_show_about))
            .on_action(cx.listener(Self::on_show_worktrees))
            .on_action(cx.listener(Self::on_new_worktree))
            .on_action(cx.listener(Self::on_show_intentions))
            .on_action(cx.listener(Self::on_tab_or_template))
            .on_action(cx.listener(Self::on_find_action))
            .on_action(cx.listener(Self::on_close_merge_dialog))
            .on_action(cx.listener(Self::on_go_mod_tidy))
            .on_action(cx.listener(Self::on_new_branch))
            .on_action(cx.listener(Self::on_stash_changes))
            .on_action(cx.listener(Self::on_unstash_changes))
            .on_action(cx.listener(Self::on_show_local_history))
            .on_action(cx.listener(Self::on_close_settings))
            .on_action(cx.listener(Self::on_run_test_at_caret))
            .on_action(cx.listener(Self::on_rename_symbol))
            .on_action(cx.listener(Self::on_open_in_new_window))
            .on_action(cx.listener(Self::on_cancel_open))
            .on_action(cx.listener(Self::on_previous_problem))
            .on_action(cx.listener(Self::on_show_problems))
            .on_action(cx.listener(Self::on_confirm_file_prompt))
            .on_action(cx.listener(Self::on_cancel_file_prompt))
            .on_action(cx.listener(Self::on_shrink_selection))
            .on_action(cx.listener(Self::on_navigate_forward))
            .on_action(cx.listener(Self::on_close_split))
            .on_action(cx.listener(Self::on_run_program))
            .on_action(cx.listener(Self::on_build_program))
            .on_action(cx.listener(Self::on_stop_program))
            .on_action(cx.listener(Self::on_edit_run_configs))
            .on_action(cx.listener(Self::on_close_run_configs))
            .capture_any_mouse_down(cx.listener(Self::autoscroll_mouse_down))
            .capture_any_mouse_down(cx.listener(Self::navigation_mouse_down))
            .capture_any_mouse_down(cx.listener(Self::gutter_mouse_down))
            .capture_any_mouse_up(cx.listener(Self::navigation_mouse_up))
            .capture_any_mouse_up(cx.listener(Self::autoscroll_mouse_up))
            .on_mouse_move(cx.listener(Self::autoscroll_mouse_move))
            .capture_key_down(cx.listener(Self::autoscroll_key_down))
            .capture_key_down(cx.listener(Self::shift_key_down))
            .on_modifiers_changed(cx.listener(Self::shift_modifiers_changed))
            .on_action(cx.listener(Self::on_next_change))
            .on_action(cx.listener(Self::on_previous_change))
            .on_action(cx.listener(Self::on_find_in_preview))
            .on_action(cx.listener(Self::on_next_match))
            .on_action(cx.listener(Self::on_previous_match))
            .on_action(cx.listener(Self::on_close_search))
            .on_action(cx.listener(Self::on_find_file))
            .on_action(cx.listener(Self::on_find_text))
            .on_action(cx.listener(Self::on_find_symbol))
            .on_action(cx.listener(Self::on_replace_text))
            .on_action(cx.listener(Self::on_palette_previous))
            .on_action(cx.listener(Self::on_palette_next))
            .on_action(cx.listener(Self::on_palette_confirm))
            .on_action(cx.listener(Self::on_palette_close))
            .relative()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme.font_family.clone())
            .child(self.render_title_bar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_activity_bar(cx))
                    // Like GoLand, the bottom panel runs the full width, under
                    // the side panel too: a log and a terminal want the room.
                    .child(
                        div().flex_1().min_w_0().h_full().child(
                            v_resizable("vertical")
                                .child(
                                    resizable_panel().child(
                                        h_resizable("main")
                                            .child(
                                                resizable_panel()
                                                    .size(px(260.))
                                                    .size_range(px(160.)..px(560.))
                                                    .visible(self.sidebar_visible)
                                                    .child(sidebar),
                                            )
                                            .child(resizable_panel().child(center)),
                                    ),
                                )
                                .child(
                                    resizable_panel()
                                        .size(px(320.))
                                        .size_range(px(120.)..px(900.))
                                        .visible(self.terminal_visible)
                                        .child(bottom),
                                ),
                        ),
                    ),
            )
            .child(self.render_status_bar(cx))
            .children(palette)
            .children(run_configs)
            .children(hunk)
            .children(project_choice)
            .children(quick_doc)
            .children(settings_page)
            .children(keymap_page)
            .children(about)
            .children(new_worktree)
            .children(merge_worktree)
            .children(merge_dialog)
            .child(zoom)
            .children(file_prompt)
            .children(autoscroll)
    }
}
