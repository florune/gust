//! Git history in the bottom panel, after GoLand's Log: branches on the
//! left, the commit graph in the middle, and the selected commit's details
//! and changed files on the right.

use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Datelike as _, Local, TimeZone as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use super::layout::BottomTab;
use crate::history::{self, Branch, ChangedFile, Commit, Details, GraphRow, Ref, Scope, Segment};
use crate::theme::palette;
use crate::ui::{UI_SMALL, UI_TEXT, file_icon, icon};

/// More commits than this are not loaded; GoLand stops early too.
const LIMIT: usize = 5000;
const ROW_HEIGHT: f32 = 26.;
const LANE_WIDTH: f32 = 14.;
/// Lanes beyond this are drawn squeezed into the last column.
const MAX_LANES: usize = 12;

pub(super) struct GitLog {
    scope: Scope,
    pub(super) branches: Vec<Branch>,
    pub(super) commits: Arc<Vec<Commit>>,
    graph: Arc<Vec<GraphRow>>,
    /// Indices of the commits the filter lets through, in order.
    pub(super) visible: Vec<usize>,
    pub(super) selected: Option<usize>,
    pub(super) details: Option<Details>,
    pub(super) filter: Entity<InputState>,
    loading: bool,
    fetching: bool,
    error: Option<SharedString>,
    /// Remote branch names, to tell them from local ones in decorations.
    remotes: HashSet<String>,
    scroll: UniformListScrollHandle,
    load: Task<()>,
    load_details: Task<()>,
    _subscription: Subscription,
}

impl GitLog {
    fn filtered(&self) -> bool {
        self.visible.len() != self.commits.len()
    }

    /// The subjects of the listed commits, top to bottom.
    #[cfg(test)]
    pub fn subjects(&self) -> Vec<String> {
        self.visible
            .iter()
            .map(|&ix| self.commits[ix].subject.clone())
            .collect()
    }
}

/// When something happened, the way a log reads best: recent times by the
/// day, older ones by date.
pub(super) fn when(time: i64) -> String {
    let Some(at) = Local.timestamp_opt(time, 0).single() else {
        return String::new();
    };
    let now = Local::now();
    let ago = now.timestamp() - time;
    if (0..60).contains(&ago) {
        return "刚刚".to_string();
    }
    if (0..3600).contains(&ago) {
        return format!("{} 分钟前", ago / 60);
    }
    let days = now.date_naive().signed_duration_since(at.date_naive()).num_days();
    match days {
        0 => at.format("今天 %H:%M").to_string(),
        1 => at.format("昨天 %H:%M").to_string(),
        _ if at.year() == now.year() => at.format("%m/%d %H:%M").to_string(),
        _ => at.format("%Y/%m/%d").to_string(),
    }
}

fn full_time(time: i64) -> String {
    Local
        .timestamp_opt(time, 0)
        .single()
        .map(|at| at.format("%Y/%m/%d %H:%M:%S").to_string())
        .unwrap_or_default()
}

/// The same color for the same person every time.
fn person_color(name: &str, cx: &App) -> Hsla {
    let colors = lane_colors(cx);
    let hash = name
        .bytes()
        .fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(byte as u32));
    colors[hash as usize % colors.len()]
}

fn lane_colors(cx: &App) -> [Hsla; 7] {
    let p = palette(cx);
    [p.blue, p.green, p.orange, p.purple, p.cyan, p.yellow, p.red]
}

fn avatar(name: &str, size: f32, cx: &App) -> Div {
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    let color = person_color(name, cx);
    div()
        .size(px(size))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(color.opacity(0.25))
        .text_color(color)
        .text_size(px(size * 0.55))
        .font_weight(FontWeight::SEMIBOLD)
        .child(initial)
}

/// The graph cell of one commit's row.
fn graph_cell(row: &GraphRow, head: bool, cx: &App) -> impl IntoElement {
    let colors = lane_colors(cx);
    let background = cx.theme().background;
    let row = row.clone();
    let lanes = row.width.min(MAX_LANES);
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let top = bounds.origin.y;
            let bottom = top + bounds.size.height;
            let middle = top + bounds.size.height / 2.;
            let x = |lane: usize| {
                bounds.origin.x + px(LANE_WIDTH / 2. + lane.min(MAX_LANES - 1) as f32 * LANE_WIDTH)
            };
            let color = |index: usize| colors[index % colors.len()];
            let node = x(row.lane);
            for segment in &row.segments {
                let mut path = PathBuilder::stroke(px(1.6));
                let shade = match *segment {
                    Segment::Through { lane, color: c } => {
                        path.move_to(point(x(lane), top));
                        path.line_to(point(x(lane), bottom));
                        c
                    }
                    Segment::In { lane, color: c } => {
                        path.move_to(point(x(lane), top));
                        if x(lane) == node {
                            path.line_to(point(node, middle));
                        } else {
                            path.curve_to(point(node, middle), point(x(lane), middle));
                        }
                        c
                    }
                    Segment::Out { lane, color: c } => {
                        path.move_to(point(node, middle));
                        if x(lane) == node {
                            path.line_to(point(node, bottom));
                        } else {
                            path.curve_to(point(x(lane), bottom), point(x(lane), middle));
                        }
                        c
                    }
                };
                if let Ok(path) = path.build() {
                    window.paint_path(path, color(shade));
                }
            }
            let radius = px(if head { 5. } else { 4. });
            let dot = Bounds::centered_at(point(node, middle), size(radius * 2., radius * 2.));
            let quad = fill(dot, if head { background } else { color(row.color) })
                .corner_radii(radius)
                .border_widths(px(if head { 2. } else { 0. }))
                .border_color(color(row.color));
            window.paint_quad(quad);
        },
    )
    .w(px(lanes as f32 * LANE_WIDTH + 6.))
    .h_full()
    .flex_shrink_0()
}

fn ref_chip(reference: &Ref, remotes: &HashSet<String>, cx: &App) -> Option<impl IntoElement> {
    let p = palette(cx);
    let theme = cx.theme();
    let (label, color, glyph) = match reference {
        Ref::Head(branch) => (branch.clone(), theme.primary, "git-branch"),
        Ref::Detached => ("HEAD".to_string(), theme.primary, "circle-dot"),
        Ref::Local(name) if remotes.contains(name) => (name.clone(), p.purple, "cloud"),
        Ref::Local(name) => (name.clone(), p.green, "git-branch"),
        Ref::Tag(tag) => (tag.clone(), p.yellow, "tag"),
    };
    Some(
        h_flex()
            .flex_shrink_0()
            .h(px(18.))
            .px_1p5()
            .gap_1()
            .rounded(px(4.))
            .bg(color.opacity(0.16))
            .text_color(color)
            .text_size(px(11.))
            .when(matches!(reference, Ref::Head(_)), |chip| {
                chip.font_weight(FontWeight::SEMIBOLD)
            })
            .child(icon(glyph).size(px(11.)))
            .child(label),
    )
}

impl Workspace {
    /// Show the log in the bottom panel, loading it the first time.
    pub(super) fn show_log(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.log.is_none() {
            let filter = cx.new(|cx| {
                InputState::new(window, cx).placeholder("搜索提交说明、作者、哈希")
            });
            let subscription = cx.subscribe(&filter, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.apply_log_filter(cx);
                }
            });
            self.log = Some(GitLog {
                scope: Scope::All,
                branches: Vec::new(),
                commits: Arc::new(Vec::new()),
                graph: Arc::new(Vec::new()),
                visible: Vec::new(),
                selected: None,
                details: None,
                filter,
                loading: true,
                fetching: false,
                error: None,
                remotes: HashSet::new(),
                scroll: UniformListScrollHandle::new(),
                load: Task::ready(()),
                load_details: Task::ready(()),
                _subscription: subscription,
            });
            self.reload_log(cx);
        }
        self.terminal_visible = true;
        self.bottom_tab = BottomTab::Log;
        cx.notify();
    }

    pub(super) fn log_visible(&self) -> bool {
        self.terminal_visible && self.bottom_tab == BottomTab::Log && self.log.is_some()
    }

    /// Read the history again, keeping the selected commit when it is still there.
    pub(super) fn reload_log(&mut self, cx: &mut Context<Self>) {
        let Some(log) = &mut self.log else {
            return;
        };
        let Some(root) = self.git.as_ref().map(|status| status.root.clone()).or_else(|| {
            // Before the first status arrives, the folder itself.
            Some(self.root.clone())
        }) else {
            return;
        };
        let scope = log.scope.clone();
        log.loading = true;
        let loading = cx.background_executor().spawn(async move {
            let commits = history::log(&root, &scope, LIMIT);
            let branches = history::branches(&root);
            let graph = commits.as_ref().map(|commits| history::graph(commits)).unwrap_or_default();
            (commits, branches, graph)
        });
        log.load = cx.spawn(async move |this, cx| {
            let (commits, branches, graph) = loading.await;
            this.update(cx, |this, cx| {
                let Some(log) = &mut this.log else {
                    return;
                };
                log.loading = false;
                match commits {
                    Ok(commits) => {
                        let selected = log
                            .selected
                            .and_then(|ix| log.commits.get(ix))
                            .map(|commit| commit.hash.clone());
                        log.error = None;
                        log.remotes = branches
                            .iter()
                            .filter(|branch| branch.remote)
                            .map(|branch| branch.name.clone())
                            .collect();
                        log.branches = branches;
                        log.commits = Arc::new(commits);
                        log.graph = Arc::new(graph);
                        log.selected = selected.and_then(|hash| {
                            log.commits.iter().position(|commit| commit.hash == hash)
                        });
                        if log.selected.is_none() {
                            log.details = None;
                        }
                        // Start on what is checked out, as GoLand does.
                        let first_selection = log.selected.is_none().then(|| {
                            log.commits
                                .iter()
                                .position(|commit| {
                                    commit.refs.iter().any(|r| matches!(r, Ref::Head(_) | Ref::Detached))
                                })
                                .unwrap_or(0)
                        });
                        if let Some(ix) = first_selection.filter(|_| !log.commits.is_empty()) {
                            this.select_commit(ix, cx);
                        }
                    }
                    Err(reason) => log.error = Some(reason.into()),
                }
                this.apply_log_filter(cx);
            })
            .ok();
        });
        cx.notify();
    }

    fn apply_log_filter(&mut self, cx: &mut Context<Self>) {
        let Some(log) = &mut self.log else {
            return;
        };
        let query = log.filter.read(cx).value().trim().to_lowercase();
        log.visible = log
            .commits
            .iter()
            .enumerate()
            .filter(|(_, commit)| {
                query.is_empty()
                    || commit.subject.to_lowercase().contains(&query)
                    || commit.author.to_lowercase().contains(&query)
                    || commit.email.to_lowercase().contains(&query)
                    || commit.hash.starts_with(&query)
            })
            .map(|(ix, _)| ix)
            .collect();
        cx.notify();
    }

    pub(super) fn set_log_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        let Some(log) = &mut self.log else {
            return;
        };
        if log.scope != scope {
            log.scope = scope;
            log.scroll.scroll_to_item(0, ScrollStrategy::Top);
            self.reload_log(cx);
        }
    }

    pub(super) fn select_commit(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(log) = &mut self.log else {
            return;
        };
        let Some(commit) = log.commits.get(ix).cloned() else {
            return;
        };
        log.selected = Some(ix);
        let root = self.git.as_ref().map_or(self.root.clone(), |s| s.root.clone());
        let loading = cx
            .background_executor()
            .spawn(async move { history::details(&root, &commit) });
        log.load_details = cx.spawn(async move |this, cx| {
            let details = loading.await;
            this.update(cx, |this, cx| {
                if let Some(log) = &mut this.log {
                    match details {
                        Ok(details) => log.details = Some(details),
                        Err(reason) => this.notice = Some(reason.into()),
                    }
                    cx.notify();
                }
            })
            .ok();
        });
        cx.notify();
    }

    fn fetch(&mut self, cx: &mut Context<Self>) {
        let Some(log) = &mut self.log else {
            return;
        };
        if log.fetching {
            return;
        }
        log.fetching = true;
        let root = self.git.as_ref().map_or(self.root.clone(), |s| s.root.clone());
        let fetching = cx.background_executor().spawn(async move { history::fetch(&root) });
        cx.spawn(async move |this, cx| {
            let result = fetching.await;
            this.update(cx, |this, cx| {
                if let Some(log) = &mut this.log {
                    log.fetching = false;
                }
                if let Err(reason) = result {
                    this.notice = Some(format!("获取失败：{reason}").into());
                }
                this.reload_log(cx);
                this.refresh_git(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub(super) fn open_commit_file(
        &mut self,
        file: Option<ChangedFile>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(log) = &self.log else {
            return;
        };
        let Some(commit) = log.selected.and_then(|ix| log.commits.get(ix)).cloned() else {
            return;
        };
        // Every file of the commit, to step through; the chosen one first shown.
        let files = log
            .details
            .as_ref()
            .filter(|details| details.hash == commit.hash)
            .map(|details| details.files.clone())
            .unwrap_or_default();
        let selected = file
            .and_then(|file| files.iter().position(|f| f.path == file.path))
            .unwrap_or(0);
        self.open_commit_diff(commit, &files, selected, window, cx);
    }

    pub(super) fn render_log_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(log) = &self.log else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        h_flex()
            .gap_1()
            .child(
                div().w(px(280.)).child(
                    Input::new(&log.filter)
                        .small()
                        .prefix(icon("search").xsmall().text_color(theme.muted_foreground))
                        .cleanable(true),
                ),
            )
            .child(
                Self::header_button("log-refresh", "refresh-cw", "刷新", cx)
                    .on_click(cx.listener(|this, _, _, cx| this.reload_log(cx))),
            )
            .child(
                Self::header_button(
                    "log-fetch",
                    if log.fetching { "loader" } else { "cloud-download" },
                    "获取远程更新 (git fetch)",
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.fetch(cx))),
            )
            .into_any_element()
    }

    fn render_branches(&self, log: &GitLog, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let row = |id: ElementId, glyph: &'static str, label: String, selected: bool| {
            h_flex()
                .id(id)
                .h(px(24.))
                .px_2()
                .mx_1()
                .gap_1p5()
                .rounded(px(5.))
                .text_size(UI_TEXT)
                .when(selected, |row| row.bg(theme.list_active))
                .when(!selected, |row| row.hover(|row| row.bg(theme.list_hover)))
                .child(icon(glyph).xsmall().text_color(theme.muted_foreground))
                .child(div().flex_1().min_w_0().truncate().child(label))
        };
        let section = |label: &'static str| {
            div()
                .px_3()
                .pt_2()
                .pb_0p5()
                .text_size(UI_SMALL)
                .text_color(theme.muted_foreground)
                .child(label)
        };
        let workspace = cx.entity().downgrade();
        let branch_row = |ix: usize, branch: &Branch| {
            let scope = Scope::Branch(branch.refname.clone());
            let selected = log.scope == scope;
            let name = branch.name.clone();
            let workspace = workspace.clone();
            let remote = branch.remote;
            let current = branch.current;
            row(
                ("log-branch", ix).into(),
                if branch.remote { "cloud" } else { "git-branch" },
                branch.name.clone(),
                selected,
            )
            .when(branch.current, |row| {
                row.font_weight(FontWeight::SEMIBOLD)
                    .child(icon("star").xsmall().text_color(palette(cx).yellow))
            })
            .when(branch.ahead > 0 || branch.behind > 0, |row| {
                let mut counts = String::new();
                if branch.ahead > 0 {
                    counts.push_str(&format!("↑{}", branch.ahead));
                }
                if branch.behind > 0 {
                    counts.push_str(&format!(" ↓{}", branch.behind));
                }
                row.child(
                    div()
                        .text_size(UI_SMALL)
                        .text_color(theme.muted_foreground)
                        .child(counts),
                )
            })
            .on_click(cx.listener(move |this, _, _, cx| this.set_log_scope(scope.clone(), cx)))
            .context_menu(move |menu, _, _| {
                let switch = workspace.clone();
                let name = name.clone();
                menu.item(
                    PopupMenuItem::new("检出此分支")
                        .disabled(remote || current)
                        .on_click(move |_, window, cx| {
                            let name = name.clone();
                            switch
                                .update(cx, |this, cx| this.switch_branch(name, window, cx))
                                .ok();
                        }),
                )
            })
        };

        let locals: Vec<(usize, &Branch)> =
            log.branches.iter().enumerate().filter(|(_, b)| !b.remote).collect();
        let remotes: Vec<(usize, &Branch)> =
            log.branches.iter().enumerate().filter(|(_, b)| b.remote).collect();
        v_flex()
            .id("log-branches")
            .size_full()
            .overflow_y_scroll()
            .py_1()
            .child(
                row("log-all".into(), "git-graph", "全部分支".to_string(), log.scope == Scope::All)
                    .on_click(cx.listener(|this, _, _, cx| this.set_log_scope(Scope::All, cx))),
            )
            .child(
                row(
                    "log-head".into(),
                    "circle-dot",
                    "HEAD（当前）".to_string(),
                    log.scope == Scope::Head,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_log_scope(Scope::Head, cx))),
            )
            .when(!locals.is_empty(), |list| list.child(section("本地")))
            .children(locals.into_iter().map(|(ix, branch)| branch_row(ix, branch)))
            .when(!remotes.is_empty(), |list| list.child(section("远程")))
            .children(remotes.into_iter().map(|(ix, branch)| branch_row(ix, branch)))
    }

    fn render_commit_row(&self, row: usize, cx: &mut Context<Self>) -> AnyElement {
        let Some(log) = &self.log else {
            return div().into_any_element();
        };
        let ix = log.visible[row];
        let commit = &log.commits[ix];
        let theme = cx.theme();
        let selected = log.selected == Some(ix);
        let head = commit
            .refs
            .iter()
            .any(|r| matches!(r, Ref::Head(_) | Ref::Detached));
        let hash = commit.hash.clone();
        let subject = commit.subject.clone();
        h_flex()
            .id(("commit", ix))
            .h(px(ROW_HEIGHT))
            .w_full()
            .pr_3()
            .gap_2()
            .text_size(UI_TEXT)
            .when(selected, |row| row.bg(theme.list_active))
            .when(!selected, |row| row.hover(|row| row.bg(theme.list_hover)))
            .map(|row| {
                // A filtered list has gaps the graph's lines cannot cross.
                match log.graph.get(ix).filter(|_| !log.filtered()) {
                    Some(graph) => row.child(graph_cell(graph, head, cx)),
                    None => row.pl_3(),
                }
            })
            .children(
                commit
                    .refs
                    .iter()
                    .filter_map(|reference| ref_chip(reference, &log.remotes, cx)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(commit.parents.len() > 1, |subject| {
                        subject.text_color(theme.muted_foreground)
                    })
                    .child(commit.subject.clone()),
            )
            .child(
                h_flex()
                    .w(px(140.))
                    .flex_shrink_0()
                    .gap_1p5()
                    .child(avatar(&commit.author, 16., cx))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(commit.author.clone()),
                    ),
            )
            .child(
                div()
                    .w(px(96.))
                    .flex_shrink_0()
                    .text_right()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .child(when(commit.time)),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.select_commit(ix, cx)))
            .context_menu(move |menu, _, _| {
                let hash = hash.clone();
                let subject = subject.clone();
                menu.item(PopupMenuItem::new("复制提交哈希").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(hash.clone()))
                }))
                .item(PopupMenuItem::new("复制提交说明").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(subject.clone()))
                }))
            })
            .into_any_element()
    }

    fn render_details(&self, log: &GitLog, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let commit = log.selected.and_then(|ix| log.commits.get(ix));
        let (Some(commit), Some(details)) =
            (commit, log.details.as_ref().filter(|d| commit.is_some_and(|c| c.hash == d.hash)))
        else {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_size(UI_TEXT)
                .text_color(theme.muted_foreground)
                .child(if commit.is_some() {
                    "正在读取…"
                } else {
                    "选择一个提交查看详情"
                })
                .into_any_element();
        };
        let (title, body) = details
            .message
            .split_once('\n')
            .map(|(title, body)| (title.to_string(), body.trim().to_string()))
            .unwrap_or_else(|| (details.message.clone(), String::new()));
        let hash = commit.hash.clone();
        let files = details.files.iter().enumerate().map(|(ix, file)| {
            let (folder, name) = match file.path.rsplit_once('/') {
                Some((folder, name)) => (folder.to_string(), name.to_string()),
                None => (String::new(), file.path.clone()),
            };
            let p = palette(cx);
            let color = match file.status {
                'A' => p.green,
                'D' => p.gray,
                'R' | 'C' => p.purple,
                _ => p.blue,
            };
            let chosen = file.clone();
            h_flex()
                .id(("commit-file", ix))
                .h(px(24.))
                .px_3()
                .gap_1p5()
                .text_size(UI_TEXT)
                .hover(|row| row.bg(theme.list_hover))
                .child(file_icon(&name, cx).small())
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(color)
                        .when(file.status == 'D', |name| name.line_through())
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(UI_SMALL)
                        .text_color(theme.muted_foreground)
                        .child(folder),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_commit_file(Some(chosen.clone()), window, cx)
                }))
        });
        v_flex()
            .id("commit-details")
            .size_full()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .p_3()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .when(!body.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(UI_TEXT)
                                .text_color(theme.muted_foreground)
                                .whitespace_normal()
                                .child(body),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .text_size(UI_SMALL)
                            .child(avatar(&commit.author, 22., cx))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(UI_TEXT)
                                            .child(commit.author.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_color(theme.muted_foreground)
                                            .child(format!(
                                                "{} · {}",
                                                commit.email,
                                                full_time(commit.time)
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .text_size(UI_SMALL)
                            .text_color(theme.muted_foreground)
                            .child(
                                h_flex()
                                    .id("commit-hash")
                                    .gap_1()
                                    .px_1p5()
                                    .rounded(px(4.))
                                    .bg(theme.secondary)
                                    .font_family(theme.mono_font_family.clone())
                                    .hover(|this| this.text_color(theme.foreground))
                                    .child(icon("copy").size(px(11.)))
                                    .child(commit.short().to_string())
                                    .tooltip(|window, cx| Tooltip::new("复制完整哈希").build(window, cx))
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(hash.clone()))
                                    }),
                            )
                            .when(details.committer != commit.author, |row| {
                                row.child(format!("由 {} 提交", details.committer))
                            }),
                    )
                    .when(!commit.refs.is_empty(), |this| {
                        this.child(
                            h_flex().flex_wrap().gap_1().children(
                                commit
                                    .refs
                                    .iter()
                                    .filter_map(|reference| ref_chip(reference, &log.remotes, cx)),
                            ),
                        )
                    }),
            )
            .child(
                h_flex()
                    .px_3()
                    .pt_2()
                    .pb_1()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .child(format!("{} 个文件", details.files.len()))
                    .child(div().flex_1())
                    .when(!details.files.is_empty(), |row| {
                        row.child(
                            div()
                                .id("commit-all-files")
                                .hover(|this| this.text_color(theme.foreground))
                                .child("查看全部改动")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_commit_file(None, window, cx)
                                })),
                        )
                    }),
            )
            .children(files)
            .into_any_element()
    }

    pub(super) fn render_log(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(log) = &self.log else {
            return div().into_any_element();
        };
        let branches = self.render_branches(log, cx);
        let details = self.render_details(log, cx);
        let theme = cx.theme();
        let list = if let Some(error) = &log.error {
            div()
                .p_3()
                .text_size(UI_TEXT)
                .text_color(theme.danger)
                .child(error.clone())
                .into_any_element()
        } else if log.visible.is_empty() {
            v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_size(UI_TEXT)
                .text_color(theme.muted_foreground)
                .child(if log.loading {
                    "正在读取历史…"
                } else if log.commits.is_empty() {
                    "还没有提交"
                } else {
                    "没有匹配的提交"
                })
                .into_any_element()
        } else {
            uniform_list(
                "log-commits",
                log.visible.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .map(|row| this.render_commit_row(row, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&log.scroll)
            .size_full()
            .into_any_element()
        };
        h_flex()
            .size_full()
            .child(
                div()
                    .w(px(220.))
                    .h_full()
                    .flex_shrink_0()
                    .bg(theme.sidebar)
                    .border_r_1()
                    .border_color(theme.border)
                    .child(branches),
            )
            .child(div().flex_1().min_w_0().h_full().child(list))
            .child(
                div()
                    .w(px(380.))
                    .h_full()
                    .flex_shrink_0()
                    .border_l_1()
                    .border_color(theme.border)
                    .child(details),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::when;

    #[test]
    fn recent_times_read_relative_and_old_ones_as_dates() {
        let now = chrono::Local::now().timestamp();
        assert_eq!(when(now - 5), "刚刚");
        assert_eq!(when(now - 600), "10 分钟前");
        assert!(when(now - 400 * 86_400).starts_with(|c: char| c.is_ascii_digit()));
    }
}
