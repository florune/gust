use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::settings::is_hidden;
use crate::ui::{UI_TEXT, file_icon, folder_icon, icon};

const ROW_HEIGHT: f32 = 24.;
const INDENT: f32 = 16.;
/// Left padding of the rows at the top level.
const LEFT: f32 = 8.;

struct Node {
    path: PathBuf,
    name: SharedString,
    is_dir: bool,
    depth: usize,
    /// Hidden by the project's settings, and shown only on request.
    hidden: bool,
}

pub enum FileTreeEvent {
    Open(PathBuf),
    /// Hide the file or folder from this project's views.
    Hide(PathBuf),
    Unhide(PathBuf),
    /// Make a file, or with `folder` a folder, in `dir`.
    New { dir: PathBuf, folder: bool },
    Rename(PathBuf),
    Delete(PathBuf),
}

/// The project's files as a flat list of the rows currently visible.
/// Directories are read only when they are expanded.
pub struct FileTree {
    root: PathBuf,
    nodes: Vec<Node>,
    expanded: HashSet<PathBuf>,
    selected: Option<PathBuf>,
    scroll: UniformListScrollHandle,
    /// Files that differ from the last commit, with the color to name them in.
    changed: HashMap<PathBuf, Hsla>,
    /// Folders holding such files.
    changed_folders: HashSet<PathBuf>,
    folder_color: Hsla,
    /// Paths from the root that the project hides.
    hidden: Vec<String>,
    /// Hidden entries are listed anyway, dimmed, so they can be shown again.
    show_hidden: bool,
}

impl EventEmitter<FileTreeEvent> for FileTree {}

impl FileTree {
    pub fn new(root: PathBuf, hidden: Vec<String>) -> Self {
        let mut tree = Self {
            root,
            nodes: Vec::new(),
            expanded: HashSet::new(),
            selected: None,
            scroll: UniformListScrollHandle::new(),
            changed: HashMap::new(),
            changed_folders: HashSet::new(),
            folder_color: Hsla::default(),
            hidden,
            show_hidden: false,
        };
        tree.reload();
        tree
    }

    /// The row last clicked.
    pub fn selected(&self) -> Option<&Path> {
        self.selected.as_deref()
    }

    pub fn set_root(&mut self, root: PathBuf, hidden: Vec<String>, cx: &mut Context<Self>) {
        self.root = root;
        self.hidden = hidden;
        self.expanded.clear();
        self.selected = None;
        self.reload();
        cx.notify();
    }

    pub fn set_changes(
        &mut self,
        files: HashMap<PathBuf, Hsla>,
        folders: HashSet<PathBuf>,
        folder_color: Hsla,
        cx: &mut Context<Self>,
    ) {
        self.changed = files;
        self.changed_folders = folders;
        self.folder_color = folder_color;
        cx.notify();
    }

    pub fn set_hidden(&mut self, hidden: Vec<String>, cx: &mut Context<Self>) {
        self.hidden = hidden;
        self.reload();
        cx.notify();
    }

    pub fn shows_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn toggle_show_hidden(&mut self, cx: &mut Context<Self>) {
        self.show_hidden = !self.show_hidden;
        self.reload();
        cx.notify();
    }

    fn relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// The names of the rows shown, for tests.
    #[cfg(test)]
    pub fn visible_paths(&self) -> Vec<String> {
        self.nodes.iter().map(|node| self.relative(&node.path)).collect()
    }

    /// Fold every folder back up.
    pub fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.reload();
        cx.notify();
    }

    /// Mark the row for `path` as the selected one, expanding its parents.
    pub fn select(&mut self, path: &Path, cx: &mut Context<Self>) {
        let mut changed = false;
        let mut parent = path.parent();
        while let Some(dir) = parent {
            if dir == self.root || !dir.starts_with(&self.root) {
                break;
            }
            changed |= self.expanded.insert(dir.to_path_buf());
            parent = dir.parent();
        }
        if changed {
            self.reload();
        }
        self.selected = Some(path.to_path_buf());
        if let Some(ix) = self.nodes.iter().position(|n| n.path == path) {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// Re-read the expanded directories from disk.
    pub fn reload(&mut self) {
        let mut nodes = Vec::new();
        self.read_dir(&self.root.clone(), 0, &mut nodes);
        self.nodes = nodes;
    }

    fn read_dir(&self, dir: &Path, depth: usize, out: &mut Vec<Node>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut children: Vec<(bool, String, PathBuf, bool)> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".git" || name == crate::settings::LOCAL_DIR {
                    return None;
                }
                let path = entry.path();
                // A hidden folder is not even read, unless asked to show it.
                let hidden = is_hidden(&self.hidden, &self.relative(&path));
                if hidden && !self.show_hidden {
                    return None;
                }
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                Some((is_dir, name, path, hidden))
            })
            .collect();
        children.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
        });
        for (is_dir, name, path, hidden) in children {
            let expanded = is_dir && self.expanded.contains(&path);
            out.push(Node {
                path: path.clone(),
                name: name.into(),
                is_dir,
                depth,
                hidden,
            });
            if expanded {
                self.read_dir(&path, depth + 1, out);
            }
        }
    }

    fn click(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(node) = self.nodes.get(ix) else {
            return;
        };
        let path = node.path.clone();
        if node.is_dir {
            if !self.expanded.remove(&path) {
                self.expanded.insert(path.clone());
            }
            self.reload();
        } else {
            cx.emit(FileTreeEvent::Open(path.clone()));
        }
        self.selected = Some(path);
        cx.notify();
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let node = &self.nodes[ix];
        let selected = self.selected.as_deref() == Some(node.path.as_path());
        let expanded = node.is_dir && self.expanded.contains(&node.path);
        let is_dir = node.is_dir;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let name_color = if node.is_dir {
            self.changed_folders
                .contains(&node.path)
                .then_some(self.folder_color)
        } else {
            self.changed.get(&node.path).copied()
        }
        .unwrap_or(theme.sidebar_foreground);

        let guide = theme.border;
        let tree = cx.entity().downgrade();
        let menu_path = node.path.clone();
        let relative = self.relative(&node.path);
        let hidden = node.hidden;
        // Hidden entries shown on request sit back from the others.
        let name_color = if hidden { muted } else { name_color };
        h_flex()
            .id(ix)
            .relative()
            .h(px(ROW_HEIGHT))
            .w_full()
            .pl(px(LEFT + node.depth as f32 * INDENT))
            .pr_2()
            .gap_1p5()
            .text_size(UI_TEXT)
            .text_color(theme.sidebar_foreground)
            .when(selected, |row| row.bg(theme.list_active))
            .when(!selected, |row| row.hover(|row| row.bg(theme.list_hover)))
            // A thin line under each open folder's contents shows what belongs to it.
            .children((0..node.depth).map(move |level| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(px(LEFT + level as f32 * INDENT + 5.5))
                    .w(px(1.))
                    .bg(guide)
            }))
            .child(if node.is_dir {
                icon(if expanded { "chevron-down" } else { "chevron-right" })
                    .xsmall()
                    .text_color(muted)
                    .into_any_element()
            } else {
                div().w(px(12.)).flex_shrink_0().into_any_element()
            })
            .child(if node.is_dir {
                folder_icon(expanded, cx).small()
            } else {
                file_icon(&node.name, cx).small()
            })
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .text_color(name_color)
                    .child(node.name.clone()),
            )
            .when(hidden, |row| row.opacity(0.6))
            .on_click(cx.listener(move |this, _, _, cx| this.click(ix, cx)))
            .context_menu(move |menu, _, _| {
                let path = menu_path.clone();
                let toggle = tree.clone();
                // New things go into the folder, or beside the file.
                let dir = if is_dir {
                    menu_path.clone()
                } else {
                    menu_path.parent().map(PathBuf::from).unwrap_or_default()
                };
                let emit = |label: &'static str, event: fn(PathBuf) -> FileTreeEvent, target: PathBuf| {
                    let tree = tree.clone();
                    PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        let target = target.clone();
                        tree.update(cx, |_, cx| cx.emit(event(target))).ok();
                    })
                };
                let menu = menu
                    .item(emit(
                        "新建文件…",
                        |dir| FileTreeEvent::New { dir, folder: false },
                        dir.clone(),
                    ))
                    .item(emit(
                        "新建文件夹…",
                        |dir| FileTreeEvent::New { dir, folder: true },
                        dir,
                    ))
                    .separator()
                    .item(emit("重命名…", FileTreeEvent::Rename, menu_path.clone()))
                    .item(emit("删除…", FileTreeEvent::Delete, menu_path.clone()))
                    .separator();
                let absolute = menu_path.to_string_lossy().into_owned();
                let relative = relative.clone();
                let reveal = menu_path.clone();
                menu.item(
                    PopupMenuItem::new(if hidden { "取消隐藏" } else { "在此项目中隐藏" })
                        .on_click(move |_, _, cx| {
                            let path = path.clone();
                            toggle
                                .update(cx, |_, cx| {
                                    cx.emit(if hidden {
                                        FileTreeEvent::Unhide(path)
                                    } else {
                                        FileTreeEvent::Hide(path)
                                    })
                                })
                                .ok();
                        }),
                )
                .separator()
                .item(PopupMenuItem::new("复制路径").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(absolute.clone()))
                }))
                .item(PopupMenuItem::new("复制相对路径").on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(relative.clone()))
                }))
                .item(PopupMenuItem::new("在资源管理器中显示").on_click(move |_, _, _| {
                    let _ = crate::proc::command("explorer")
                        .arg(format!("/select,{}", reveal.display()))
                        .spawn();
                }))
            })
    }
}

impl Render for FileTree {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        uniform_list(
            "file-tree",
            self.nodes.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full()
    }
}
