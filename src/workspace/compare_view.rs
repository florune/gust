//! Side-by-side comparison, after GoLand: the old version on the left and
//! the new on the right, each a read-only editor that scrolls sideways on its
//! own, with the changed regions shaded and joined by bands between them.
//! Lines are never wrapped, so line 365 is always one row.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::input::{Editor, EditorState, TabSize, TextDecoration, TextDecorationCollection};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use crate::compare::{ChunkKind, Comparison, compare};
use crate::history::{self, ChangedFile, Commit};
use crate::lang;
use crate::theme::palette;
use crate::ui::{UI_SMALL, UI_TEXT, file_icon, icon};

actions!(compare, [NextChange, PreviousChange]);

pub(super) fn init(cx: &mut App) {
    let workspace = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("f7", NextChange, workspace),
        KeyBinding::new("shift-f7", PreviousChange, workspace),
    ]);
}

/// Larger files are not compared.
const MAX_BYTES: usize = 8 * 1024 * 1024;
/// Width of the strip between the two sides, where the bands are drawn.
const STRIP: f32 = 40.;
/// Lines left above a change that navigation brings into view.
const CONTEXT_ABOVE: f32 = 4.;

/// Which two versions are compared.
#[derive(Clone)]
pub(super) enum Source {
    /// The last commit against the files on disk.
    WorkingTree,
    /// A commit against its first parent.
    Commit(Commit),
    /// A file as it is on disk against its unsaved edits.
    Disk { file: PathBuf, mine: String },
    /// A version kept in local history against the file's current text.
    Version { file: PathBuf, version: PathBuf, current: String },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Entry {
    pub path: String,
    pub old_path: Option<String>,
    /// `A`, `M`, `D`, `R`, `C`.
    pub status: char,
}

impl Entry {
    fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

impl From<&ChangedFile> for Entry {
    fn from(file: &ChangedFile) -> Self {
        Self {
            path: file.path.clone(),
            old_path: file.old_path.clone(),
            status: file.status,
        }
    }
}

impl From<&crate::git::Change> for Entry {
    fn from(change: &crate::git::Change) -> Self {
        use crate::git::ChangeKind;
        Self {
            path: change.path.clone(),
            old_path: change.old_path.clone(),
            status: match change.kind {
                ChangeKind::Added | ChangeKind::Untracked => 'A',
                ChangeKind::Deleted => 'D',
                ChangeKind::Renamed => 'R',
                ChangeKind::Modified | ChangeKind::Conflict => 'M',
            },
        }
    }
}

/// The one tab that compares versions, reused for whatever was last asked for.
pub(super) struct DiffTab {
    pub title: SharedString,
    pub source: Source,
    pub entries: Vec<Entry>,
    pub selected: usize,
    view: Option<CompareView>,
    load: Task<()>,
}

/// The two sides of the file being compared.
struct CompareView {
    entry: Entry,
    left: Entity<EditorState>,
    right: Entity<EditorState>,
    comparison: Rc<Comparison>,
    /// The new side's text, to tell whether a reload changed anything.
    new_text: String,
    left_label: SharedString,
    right_label: SharedString,
    /// Why the text is not shown: a binary or very large file.
    note: Option<SharedString>,
    /// Scroll positions, left and right, after the last sync.
    synced: (Pixels, Pixels),
    /// A side moved by the other, and the frame it was moved in. A move takes
    /// effect when the side is next drawn, so until then what the side
    /// reports is its old position, which must not be synced back.
    moved: [Option<u64>; 2],
    /// Frames drawn so far, counted by the strip between the sides.
    frames: Rc<Cell<u64>>,
    _decorations: Vec<TextDecorationCollection>,
    _subscriptions: Vec<Subscription>,
}

impl DiffTab {
    fn entry(&self) -> Option<&Entry> {
        self.entries.get(self.selected)
    }

    /// Lines added and removed in the file shown.
    pub fn counts(&self) -> (usize, usize) {
        self.view
            .as_ref()
            .map_or((0, 0), |view| (view.comparison.added, view.comparison.removed))
    }

    /// The two editors, old and new, once the file has been read.
    #[cfg(test)]
    pub fn sides(&self) -> Option<(Entity<EditorState>, Entity<EditorState>)> {
        self.view
            .as_ref()
            .map(|view| (view.left.clone(), view.right.clone()))
    }

    /// The working-tree file shown, if that is what the tab compares.
    pub fn working_path(&self) -> Option<String> {
        match self.source {
            Source::WorkingTree => self.entry().map(|entry| entry.path.clone()),
            Source::Commit(_) | Source::Disk { .. } | Source::Version { .. } => None,
        }
    }
}

/// Text read for one side, or why it cannot be shown.
type Side = Result<String, SharedString>;

fn side(bytes: Option<Vec<u8>>) -> Side {
    let bytes = bytes.unwrap_or_default();
    if bytes.len() > MAX_BYTES {
        return Err("文件过大，无法比较".into());
    }
    if bytes.contains(&0) {
        return Err("二进制文件".into());
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(if text.contains("\r\n") {
        text.replace("\r\n", "\n")
    } else {
        text.into_owned()
    })
}

/// Read both versions of `entry`, and the labels above them.
fn read_versions(root: &Path, source: &Source, entry: &Entry) -> (Side, Side, String, String) {
    let old_path = entry.old_path.as_deref().unwrap_or(&entry.path);
    match source {
        Source::WorkingTree => {
            let old = (entry.status != 'A')
                .then(|| history::file_at(root, "HEAD", old_path))
                .flatten();
            let new = (entry.status != 'D')
                .then(|| std::fs::read(root.join(&entry.path)).ok())
                .flatten();
            (side(old), side(new), "HEAD".into(), "工作区".into())
        }
        Source::Version { version, current, .. } => (
            crate::local_history::load(version).ok_or_else(|| "无法读取该版本".into()),
            Ok(current.clone()),
            "本地历史".into(),
            "当前".into(),
        ),
        Source::Disk { file, mine } => (
            side(std::fs::read(file).ok()),
            Ok(mine.clone()),
            "磁盘版本".into(),
            "未保存的修改".into(),
        ),
        Source::Commit(commit) => {
            let parent = commit.parents.first();
            let old = parent
                .filter(|_| entry.status != 'A')
                .and_then(|parent| history::file_at(root, parent, old_path));
            let new = (entry.status != 'D')
                .then(|| history::file_at(root, &commit.hash, &entry.path))
                .flatten();
            let parent_label = parent.map_or("（无）".to_string(), |p| p[..p.len().min(8)].to_string());
            (side(old), side(new), parent_label, commit.short().to_string())
        }
    }
}

/// Where each line of an editor is drawn: its top edge in window coordinates.
#[derive(Clone, Copy)]
struct Rows {
    top: Pixels,
    height: Pixels,
}

impl Rows {
    fn of(editor: &EditorState) -> Option<Self> {
        let bounds = editor.text_bounds()?;
        let height = editor.line_height().filter(|h| *h > px(0.))?;
        // The text's bounds already move with the scroll position.
        Some(Self {
            top: bounds.origin.y,
            height,
        })
    }

    fn y(&self, line: usize) -> Pixels {
        self.top + self.height * line as f32
    }
}

impl Workspace {
    fn diff_root(&self) -> std::path::PathBuf {
        self.git
            .as_ref()
            .map_or(self.root.clone(), |status| status.root.clone())
    }

    /// Compare the working tree's changed files, starting with `path`.
    pub(super) fn open_diff(
        &mut self,
        path: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries: Vec<Entry> = self
            .git
            .iter()
            .flat_map(|status| &status.changes)
            .map(Entry::from)
            .collect();
        let selected = path
            .as_ref()
            .and_then(|path| entries.iter().position(|entry| &entry.path == path))
            .unwrap_or(0);
        self.show_compare(Source::WorkingTree, entries, selected, window, cx);
    }

    /// Compare what a commit changed, starting with the file at `selected`.
    pub(super) fn open_commit_diff(
        &mut self,
        commit: Commit,
        files: &[ChangedFile],
        selected: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entries = files.iter().map(Entry::from).collect();
        self.show_compare(Source::Commit(commit), entries, selected, window, cx);
    }

    pub(super) fn show_compare(
        &mut self,
        source: Source,
        entries: Vec<Entry>,
        selected: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.diff.is_none() {
            // A new diff tab opens after the files already open.
            self.diff_slot = self.docs.len();
        }
        self.diff = Some(DiffTab {
            title: SharedString::default(),
            source,
            entries,
            selected,
            view: None,
            load: Task::ready(()),
        });
        self.load_compare(false, cx);
        self.show_diff(window, cx);
    }

    pub(super) fn select_compare_entry(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = &mut self.diff else {
            return;
        };
        if ix >= tab.entries.len() || ix == tab.selected {
            return;
        }
        tab.selected = ix;
        self.load_compare(false, cx);
        self.show_diff(window, cx);
    }

    /// Read the selected file's two versions and show them; `keep_scroll`
    /// leaves the view where it was when the same file is read again.
    pub(super) fn load_compare(&mut self, keep_scroll: bool, cx: &mut Context<Self>) {
        let root = self.diff_root();
        let window = self.window;
        let Some(tab) = &mut self.diff else {
            return;
        };
        let Some(entry) = tab.entry().cloned() else {
            tab.title = "没有改动".into();
            tab.view = None;
            cx.notify();
            return;
        };
        let short = match &tab.source {
            Source::Commit(commit) => format!("{} · ", commit.short()),
            Source::WorkingTree => String::new(),
            Source::Disk { .. } => "冲突 · ".to_string(),
            Source::Version { .. } => "本地历史 · ".to_string(),
        };
        tab.title = format!("{short}{}", entry.name()).into();
        let source = tab.source.clone();
        let reading = cx.background_executor().spawn({
            let entry = entry.clone();
            async move { read_versions(&root, &source, &entry) }
        });
        tab.load = cx.spawn(async move |this, cx| {
            let (old, new, left_label, right_label) = reading.await;
            let _ = cx.update_window(window, |_, window, cx| {
                this.update(cx, |this, cx| {
                    this.finish_compare(entry, old, new, (left_label, right_label), keep_scroll, window, cx)
                })
            });
        });
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_compare(
        &mut self,
        entry: Entry,
        old: Side,
        new: Side,
        labels: (String, String),
        keep_scroll: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_right = self.diff_active;
        let Some(tab) = &mut self.diff else {
            return;
        };
        if tab.entry() != Some(&entry) {
            return;
        }
        let note = old.as_ref().err().or(new.as_ref().err()).cloned();
        let old = old.unwrap_or_default();
        let new = new.unwrap_or_default();
        // The same file read again with nothing changed: keep everything as is.
        if keep_scroll
            && let Some(view) = &tab.view
            && view.entry == entry
            && view.new_text == new
        {
            return;
        }
        let scroll = tab
            .view
            .as_ref()
            .filter(|view| keep_scroll && view.entry == entry)
            .map(|view| {
                (
                    view.left.read(cx).scroll_offset(),
                    view.right.read(cx).scroll_offset(),
                )
            });

        let comparison = Rc::new(if note.is_some() {
            Comparison::default()
        } else {
            compare(&old, &new)
        });
        let language = lang::language_for(Path::new(&entry.path));
        let make = |text: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .tab_size(TabSize {
                        tab_size: 4,
                        hard_tabs: matches!(language, "go" | "make"),
                    })
                    .soft_wrap(false)
                    .default_value(text)
            })
        };
        let left = make(old, window, cx);
        let right = make(new.clone(), window, cx);

        // The words that changed inside modified lines stand out more.
        let word = |color: Hsla| HighlightStyle {
            background_color: Some(color),
            ..Default::default()
        };
        let colors = palette(cx);
        let strong = colors.blue.opacity(if cx.theme().is_dark() { 0.38 } else { 0.28 });
        let decorations = vec![
            left.update(cx, |editor, cx| {
                let words = comparison
                    .old_words
                    .iter()
                    .map(|range| TextDecoration::new(range.clone(), word(strong)))
                    .collect();
                editor.create_decorations_collection(words, cx)
            }),
            right.update(cx, |editor, cx| {
                let words = comparison
                    .new_words
                    .iter()
                    .map(|range| TextDecoration::new(range.clone(), word(strong)))
                    .collect();
                editor.create_decorations_collection(words, cx)
            }),
        ];
        let subscriptions = vec![
            cx.observe(&left, |this, _, cx| this.sync_compare(true, cx)),
            cx.observe(&right, |this, _, cx| this.sync_compare(false, cx)),
        ];
        if let Some((left_scroll, right_scroll)) = scroll {
            left.update(cx, |editor, cx| editor.set_scroll_offset(left_scroll, cx));
            right.update(cx, |editor, cx| editor.set_scroll_offset(right_scroll, cx));
        }
        let first_change = comparison.chunks.first().map(|chunk| chunk.new.start);
        tab.view = Some(CompareView {
            entry,
            left,
            right: right.clone(),
            comparison,
            new_text: new,
            left_label: labels.0.into(),
            right_label: labels.1.into(),
            note,
            synced: (px(0.), px(0.)),
            moved: [None, None],
            frames: Rc::new(Cell::new(0)),
            _decorations: decorations,
            _subscriptions: subscriptions,
        });
        if focus_right {
            right.update(cx, |editor, cx| editor.focus(window, cx));
        }
        // A newly opened file starts at its first change, as GoLand does.
        if scroll.is_none()
            && let Some(line) = first_change
        {
            self.scroll_compare_to(line, window, cx);
        }
        cx.notify();
    }

    /// Bring line `line` of the new side into view, with a little room above
    /// it, and the old side to the lines beside it.
    fn scroll_compare_to(&mut self, line: usize, window: &mut Window, cx: &mut Context<Self>) {
        let top = (line as f32 - CONTEXT_ABOVE).max(0.);
        cx.spawn_in(window, async move |this, cx| {
            // The line height is known once the editors have been laid out.
            for _ in 0..30 {
                let done = this.update(cx, |this, cx| this.scroll_compare_lines(top, cx));
                if !matches!(done, Ok(false)) {
                    break;
                }
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
            }
        })
        .detach();
    }

    /// Put line `top` of the new side at the top, and the old side beside it.
    fn scroll_compare_lines(&mut self, top: f32, cx: &mut Context<Self>) -> bool {
        let Some(view) = self.diff.as_mut().and_then(|tab| tab.view.as_mut()) else {
            return true;
        };
        let Some(height) = view.right.read(cx).line_height().filter(|h| *h > px(0.)) else {
            return false;
        };
        let right_y = height * top;
        let left_y = height * view.comparison.map_line(top, false).max(0.);
        let frame = view.frames.get();
        for (editor, y) in [(&view.left, left_y), (&view.right, right_y)] {
            let x = editor.read(cx).scroll_offset().x;
            editor.update(cx, |editor, cx| editor.set_scroll_offset(point(x, -y), cx));
        }
        view.moved = [Some(frame), Some(frame)];
        view.synced = (left_y, right_y);
        true
    }

    /// The working tree changed: list its files again and reread the one shown.
    pub(super) fn refresh_working_diff(&mut self, cx: &mut Context<Self>) {
        let entries: Vec<Entry> = self
            .git
            .iter()
            .flat_map(|status| &status.changes)
            .map(Entry::from)
            .collect();
        let Some(tab) = &mut self.diff else {
            return;
        };
        if !matches!(tab.source, Source::WorkingTree) {
            return;
        }
        let current = tab.entry().map(|entry| entry.path.clone());
        tab.selected = current
            .and_then(|path| entries.iter().position(|entry| entry.path == path))
            .unwrap_or(0);
        tab.entries = entries;
        self.load_compare(true, cx);
    }

    /// One side scrolled: move the other to the lines beside it.
    fn sync_compare(&mut self, from_left: bool, cx: &mut Context<Self>) {
        let Some(view) = self.diff.as_mut().and_then(|tab| tab.view.as_mut()) else {
            return;
        };
        let (source, target) = if from_left {
            (&view.left, &view.right)
        } else {
            (&view.right, &view.left)
        };
        let scrolled = -source.read(cx).scroll_offset().y;
        let side = if from_left { 0 } else { 1 };
        if let Some(frame) = view.moved[side] {
            // Moved by the other side: wait until that has been drawn, then
            // take where it ended up (clamped, perhaps) as the new baseline.
            if view.frames.get() <= frame + 1 {
                return;
            }
            // Drawn since: from here on what it reports is real, and a
            // difference from where it was put is the user scrolling it.
            view.moved[side] = None;
        }
        let last = if from_left { view.synced.0 } else { view.synced.1 };
        if scrolled == last {
            return;
        }
        let Some(height) = source.read(cx).line_height().filter(|h| *h > px(0.)) else {
            return;
        };
        let line = view.comparison.map_line(scrolled / height, from_left);
        let target_y = height * line.max(0.);
        let x = target.read(cx).scroll_offset().x;
        target.update(cx, |editor, cx| editor.set_scroll_offset(point(x, -target_y), cx));
        view.moved[1 - side] = Some(view.frames.get());
        view.synced = if from_left {
            (scrolled, target_y)
        } else {
            (target_y, scrolled)
        };
    }

    fn step_change(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.diff.as_ref().and_then(|tab| tab.view.as_ref()) else {
            return;
        };
        let right = view.right.read(cx);
        let Some(height) = right.line_height().filter(|h| *h > px(0.)) else {
            return;
        };
        let top = -right.scroll_offset().y / height + CONTEXT_ABOVE;
        let starts = view.comparison.chunks.iter().map(|chunk| chunk.new.start as f32);
        let target = if forward {
            starts.into_iter().find(|start| *start > top + 0.5)
        } else {
            starts.rev().find(|start| *start < top - 0.5)
        };
        if let Some(line) = target {
            self.scroll_compare_to(line as usize, window, cx);
        }
    }

    pub(super) fn on_next_change(&mut self, _: &NextChange, window: &mut Window, cx: &mut Context<Self>) {
        self.step_change(true, window, cx);
    }

    pub(super) fn on_previous_change(
        &mut self,
        _: &PreviousChange,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_change(false, window, cx);
    }

    /// Bring the diff tab to the front, the keyboard with it.
    pub(super) fn show_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = &self.diff else {
            return;
        };
        self.diff_active = true;
        match &tab.view {
            Some(view) => view.right.update(cx, |editor, cx| editor.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
        cx.notify();
    }

    pub(super) fn close_diff(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.diff = None;
        self.diff_active = false;
        self.focus_editor(window, cx);
        cx.notify();
    }

    fn render_compare_entries(&self, tab: &DiffTab, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let colors = palette(cx);
        v_flex()
            .id("compare-files")
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .overflow_y_scroll()
            .py_1()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .children(tab.entries.iter().enumerate().map(|(ix, entry)| {
                let (folder, name) = match entry.path.rsplit_once('/') {
                    Some((folder, name)) => (folder.to_string(), name.to_string()),
                    None => (String::new(), entry.path.clone()),
                };
                let color = match entry.status {
                    'A' => colors.green,
                    'D' => colors.gray,
                    'R' | 'C' => colors.purple,
                    _ => colors.blue,
                };
                let selected = ix == tab.selected;
                h_flex()
                    .id(("compare-file", ix))
                    .h(px(24.))
                    .px_2()
                    .mx_1()
                    .gap_1p5()
                    .rounded(px(5.))
                    .text_size(UI_TEXT)
                    .when(selected, |row| row.bg(theme.list_active))
                    .when(!selected, |row| row.hover(|row| row.bg(theme.list_hover)))
                    .child(file_icon(&name, cx).small())
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(color)
                            .when(entry.status == 'D', |name| name.line_through())
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
                        this.select_compare_entry(ix, window, cx)
                    }))
            }))
            .into_any_element()
    }

    pub(super) fn render_diff(&self, tab: &DiffTab, cx: &mut Context<Self>) -> AnyElement {
        let entries = (tab.entries.len() > 1).then(|| self.render_compare_entries(tab, cx));
        let theme = cx.theme();
        let colors = palette(cx);
        let (added, removed) = tab.counts();
        let entry = tab.entry().cloned();
        let open_path = match &tab.source {
            Source::Disk { file, .. } | Source::Version { file, .. } => Some(file.clone()),
            _ => entry
                .as_ref()
                .filter(|entry| entry.status != 'D')
                .map(|entry| self.diff_root().join(&entry.path))
                .filter(|path| path.is_file()),
        };
        // The open file whose disk version is compared, to settle the conflict.
        let conflict = match &tab.source {
            Source::Disk { file, .. } => self.docs.iter().position(|doc| &doc.path == file),
            _ => None,
        };
        let label = match (&tab.source, &entry) {
            (Source::Commit(commit), Some(entry)) => {
                format!("{} · {} · {}", commit.short(), entry.path, commit.subject)
            }
            (Source::WorkingTree, Some(entry)) => format!("{} · 相对上次提交", entry.path),
            (Source::Disk { .. }, Some(entry)) => {
                format!("{} · 磁盘版本与未保存的修改", entry.path)
            }
            (Source::Version { .. }, Some(entry)) => format!("{} · 本地历史版本与当前内容", entry.path),
            (_, None) => "没有改动".to_string(),
        };
        let header_button = |id: &'static str, glyph: &'static str, tip: &'static str| {
            div()
                .id(id)
                .flex_shrink_0()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .hover(|this| this.bg(theme.list_hover).text_color(theme.foreground))
                .child(icon(glyph).small())
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        };
        let header = h_flex()
            .h(px(32.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .text_size(UI_SMALL)
            .text_color(theme.muted_foreground)
            .border_b_1()
            .border_color(theme.border)
            .child(div().flex_1().min_w_0().truncate().child(label))
            .child(div().flex_shrink_0().text_color(colors.green).child(format!("+{added}")))
            .child(div().flex_shrink_0().text_color(colors.red).child(format!("−{removed}")))
            .child(
                header_button("change-previous", "arrow-up", "上一处改动 (Shift+F7)").on_click(
                    cx.listener(|this, _, window, cx| this.step_change(false, window, cx)),
                ),
            )
            .child(
                header_button("change-next", "arrow-down", "下一处改动 (F7)").on_click(
                    cx.listener(|this, _, window, cx| this.step_change(true, window, cx)),
                ),
            )
            .when_some(
                match &tab.source {
                    Source::Version { file, version, .. } => Some((file.clone(), version.clone())),
                    _ => None,
                },
                |row, (file, version)| {
                    use gpui_kit::component::button::{Button, ButtonVariants as _};
                    row.child(
                        Button::new("version-restore")
                            .xsmall()
                            .primary()
                            .label("恢复此版本")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.restore_version(&file, &version, window, cx)
                            })),
                    )
                },
            )
            .when_some(conflict, |row, ix| {
                use gpui_kit::component::button::{Button, ButtonVariants as _};
                row.child(
                    Button::new("disk-take-all")
                        .xsmall()
                        .label("全部使用磁盘版本")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.take_disk(ix, window, cx)
                        })),
                )
                .child(
                    Button::new("disk-keep-mine")
                        .xsmall()
                        .primary()
                        .label("保留我的修改")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.keep_mine(ix, window, cx)
                        })),
                )
            })
            .when_some(open_path, |row, path| {
                row.child(
                    div()
                        .id("diff-open-file")
                        .flex_shrink_0()
                        .hover(|this| this.text_color(theme.foreground))
                        .child("打开文件")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open(path.clone(), window, cx)
                        })),
                )
            });

        let body = match &tab.view {
            None => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_size(UI_TEXT)
                .text_color(theme.muted_foreground)
                .child(if entry.is_some() { "正在读取…" } else { "没有改动" })
                .into_any_element(),
            Some(view) if view.note.is_some() => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_size(UI_TEXT)
                .text_color(theme.muted_foreground)
                .child(view.note.clone().unwrap())
                .into_any_element(),
            Some(view) => {
                let mergeable = matches!(tab.source, Source::Disk { .. });
                self.render_sides(view, mergeable, cx)
            }
        };
        v_flex()
            .size_full()
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(entries)
                    .child(div().flex_1().min_w_0().h_full().child(body)),
            )
            .into_any_element()
    }

    fn render_sides(&self, view: &CompareView, mergeable: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let colors = palette(cx);
        let dark = theme.is_dark();
        let tint = move |color: Hsla| color.opacity(if dark { 0.16 } else { 0.12 });
        let band = move |color: Hsla| color.opacity(if dark { 0.28 } else { 0.22 });
        let kind_color = move |kind: ChunkKind| match kind {
            ChunkKind::Added => colors.green,
            ChunkKind::Removed => colors.gray,
            ChunkKind::Modified => colors.blue,
        };

        // Shaded rows under one side's text; a line where the other side's
        // lines would go when this side has none.
        let underlay = |editor: Entity<EditorState>, comparison: Rc<Comparison>, old: bool| {
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    let Some(rows) = Rows::of(editor.read(cx)) else {
                        return;
                    };
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for chunk in &comparison.chunks {
                            let lines = if old { &chunk.old } else { &chunk.new };
                            let color = kind_color(chunk.kind);
                            let (top, bottom) = (rows.y(lines.start), rows.y(lines.end));
                            if bottom < bounds.top() || top > bounds.bottom() {
                                continue;
                            }
                            let quad = if lines.is_empty() {
                                fill(
                                    Bounds::new(
                                        point(bounds.left(), top - px(1.)),
                                        size(bounds.size.width, px(2.)),
                                    ),
                                    color.opacity(0.7),
                                )
                            } else {
                                fill(
                                    Bounds::new(
                                        point(bounds.left(), top),
                                        size(bounds.size.width, bottom - top),
                                    ),
                                    tint(color),
                                )
                            };
                            window.paint_quad(quad);
                        }
                    });
                },
            )
            .absolute()
            .inset_0()
        };

        let strip = {
            let (left, right) = (view.left.clone(), view.right.clone());
            let comparison = view.comparison.clone();
            let frames = view.frames.clone();
            canvas(
                move |_, _, _| frames.set(frames.get() + 1),
                move |bounds, _, window, cx| {
                    let (Some(l), Some(r)) = (Rows::of(left.read(cx)), Rows::of(right.read(cx)))
                    else {
                        return;
                    };
                    let (x0, x1) = (bounds.left(), bounds.right());
                    let mid = x0 + (x1 - x0) / 2.;
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for chunk in &comparison.chunks {
                            let (a, b) = (l.y(chunk.old.start), l.y(chunk.old.end));
                            let (c, d) = (r.y(chunk.new.start), r.y(chunk.new.end));
                            if a.max(c) < bounds.top() - px(200.)
                                || a.min(c) > bounds.bottom() + px(200.)
                            {
                                continue;
                            }
                            // An S-shaped band from the left lines to the right ones.
                            let mut path = PathBuilder::fill();
                            path.move_to(point(x0, a));
                            path.curve_to(point(mid, (a + c) / 2.), point(mid, a));
                            path.curve_to(point(x1, c), point(mid, c));
                            path.line_to(point(x1, d.max(c + px(1.))));
                            path.curve_to(point(mid, (b + d) / 2.), point(mid, d));
                            path.curve_to(point(x0, b.max(a + px(1.))), point(mid, b));
                            path.close();
                            if let Ok(path) = path.build() {
                                window.paint_path(path, band(kind_color(chunk.kind)));
                            }
                            // An arrow that takes the left side's lines.
                            if mergeable {
                                let y = (a + b.max(a + l.height)) / 2.;
                                let x = x0 + px(6.);
                                let mut arrow = PathBuilder::fill();
                                arrow.move_to(point(x, y - px(6.)));
                                arrow.line_to(point(x + px(9.), y));
                                arrow.line_to(point(x, y + px(6.)));
                                arrow.close();
                                if let Ok(arrow) = arrow.build() {
                                    window.paint_path(arrow, kind_color(chunk.kind));
                                }
                            }
                        }
                    });
                },
            )
            .size_full()
        };
        let strip = div()
            .id("compare-strip")
            .w(px(STRIP))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .when(mergeable, |strip| {
                strip.cursor_pointer().on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.take_disk_chunk_at(event.position.y, window, cx)
                    }),
                )
            })
            .child(strip);

        let pane = |editor: &Entity<EditorState>, old: bool| {
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(underlay(editor.clone(), view.comparison.clone(), old))
                .child(
                    Editor::new(editor)
                        .readonly(true)
                        .appearance(false)
                        .bordered(false)
                        .size_full(),
                )
        };
        let label = |text: SharedString, icon_name: &'static str| {
            h_flex()
                .flex_1()
                .min_w_0()
                .px_3()
                .gap_1p5()
                .child(icon(icon_name).xsmall())
                .child(div().truncate().child(text))
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(26.))
                    .flex_shrink_0()
                    .text_size(UI_SMALL)
                    .text_color(theme.muted_foreground)
                    .bg(theme.sidebar)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(label(view.left_label.clone(), "git-commit-horizontal"))
                    .child(div().w(px(STRIP)).flex_shrink_0())
                    .child(label(view.right_label.clone(), "git-commit-horizontal")),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(pane(&view.left, true))
                    .child(strip)
                    .child(pane(&view.right, false)),
            )
            .into_any_element()
    }

    /// In a disk compare, the change beside height `y` of the strip.
    fn take_disk_chunk_at(&mut self, y: Pixels, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = self.diff.as_ref().and_then(|tab| tab.view.as_ref()) else {
            return;
        };
        let (Some(l), Some(r)) = (Rows::of(view.left.read(cx)), Rows::of(view.right.read(cx))) else {
            return;
        };
        let found = view.comparison.chunks.iter().position(|chunk| {
            let top = l.y(chunk.old.start).min(r.y(chunk.new.start));
            let bottom = l.y(chunk.old.end).max(r.y(chunk.new.end)).max(top + l.height);
            (top - px(2.)..bottom + px(2.)).contains(&y)
        });
        if let Some(ix) = found {
            self.take_disk_chunk(ix, window, cx);
        }
    }

    /// In a disk compare, take the disk's lines of change `ix` into the open file.
    pub(super) fn take_disk_chunk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = &self.diff else {
            return;
        };
        let Source::Disk { file, .. } = &tab.source else {
            return;
        };
        let Some(view) = &tab.view else {
            return;
        };
        let Some(chunk) = view.comparison.chunks.get(ix).cloned() else {
            return;
        };
        let disk = view.left.read(cx).text().to_string();
        let shown = view.right.read(cx).text().to_string();
        let file = file.clone();
        let Some(doc) = self.docs.iter().find(|doc| doc.path == file) else {
            return;
        };
        let editor = doc.editor.clone();
        let mine = editor.read(cx).text().to_string();
        // The compare is behind the editor: let it catch up first.
        if mine != shown {
            self.refresh_disk_compare(&file, cx);
            return;
        }
        let merged = crate::compare::take_lines(&disk, &mine, &chunk);
        super::replace_text(&editor, &mine, &merged, window, cx);
        self.refresh_dirty(&editor, cx);
    }
}
