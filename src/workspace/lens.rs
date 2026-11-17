//! Counts after declarations: how many places use each function, method,
//! and type of a Go file, and how many types implement each interface and
//! its methods. A click lists them, as Find Usages does.

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::{ActiveTheme as _, RopeExt as _};
use gpui_kit::*;

use super::Workspace;
use crate::finder;
use crate::goindex::{self, Kind, Lens};

/// Room between the end of the line and the note, in widths of the text size.
pub(super) const GAP: f32 = 2.;

/// What the note after a declaration says.
pub(super) fn label(lens: &Lens) -> String {
    let usages = match lens.usages {
        0 => "无用法".to_string(),
        n => format!("{n} 处用法"),
    };
    match lens.implementations {
        Some(n) => format!("{n} 个实现 · {usages}"),
        None => usages,
    }
}

/// The counts of one open file, and the project state they were made from.
#[derive(Default)]
pub(super) struct Lenses {
    pub by_key: Rc<HashMap<String, Lens>>,
    /// The value of the workspace's generation they were counted at.
    counted_at: Option<u64>,
    task: Option<Task<()>>,
}

impl Workspace {
    /// Count again for the files on screen whose counts are out of date.
    pub(super) fn refresh_lenses(&mut self, cx: &mut Context<Self>) {
        if !self.app.code_lens {
            return;
        }
        let Some(ix) = self.active else {
            return;
        };
        let generation = self.lens_generation;
        let Some(doc) = self.docs.get_mut(ix) else {
            return;
        };
        if doc.language != "go" || doc.lenses.counted_at == Some(generation) {
            return;
        }
        doc.lenses.counted_at = Some(generation);
        let path = doc.path.clone();
        let open: HashMap<PathBuf, String> = self
            .docs
            .iter()
            .filter(|doc| doc.language == "go")
            .map(|doc| (doc.path.clone(), doc.editor.read(cx).text().to_string()))
            .collect();
        let Some(source) = open.get(&path).cloned() else {
            return;
        };
        let root = self.root.clone();
        let known = self.project_files.clone();
        let hidden = self.settings.hidden.clone();
        let index = self.index.clone();
        let task = cx.spawn(async move |this, cx| {
            // Switching through tabs quickly counts only where it stops.
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let counting = cx.background_executor().spawn(async move {
                let files = known.unwrap_or_else(|| Arc::new(finder::project_files(&root, &hidden)));
                let absolute: Vec<PathBuf> = files.iter().map(|file| root.join(file)).collect();
                let found = goindex::lenses(&index, &absolute, &open, &path, &source);
                (files, path, found)
            });
            let (files, path, found) = counting.await;
            this.update(cx, |this, cx| {
                this.project_files.get_or_insert(files);
                if let Some(doc) = this.docs.iter_mut().find(|doc| doc.path == path) {
                    doc.lenses.by_key =
                        Rc::new(found.into_iter().map(|lens| (lens.key.clone(), lens)).collect());
                    cx.notify();
                }
            })
            .ok();
        });
        if let Some(doc) = self.docs.get_mut(ix) {
            doc.lenses.task = Some(task);
        }
    }

    /// Counts made before now no longer hold: a file was saved or changed on disk.
    pub(super) fn lenses_out_of_date(&mut self, cx: &mut Context<Self>) {
        self.lens_generation += 1;
        cx.notify();
    }

    /// The note of each declaration of the file at `ix`: row, column of its name, text.
    pub(super) fn lens_notes(&self, ix: usize) -> Vec<(u32, u32, SharedString)> {
        if !self.app.code_lens {
            return Vec::new();
        }
        let Some(doc) = self.docs.get(ix) else {
            return Vec::new();
        };
        if doc.lenses.by_key.is_empty() {
            return Vec::new();
        }
        // Rows come from the outline, which follows edits; counts wait for a save.
        doc.outline
            .iter()
            .filter(|item| matches!(item.kind, Kind::Func | Kind::Method | Kind::Type))
            .filter_map(|item| {
                let lens = doc.lenses.by_key.get(&item.full_name())?;
                Some((item.row, item.col, label(lens).into()))
            })
            .collect()
    }

    /// A layer over the editor of the file at `ix` with the notes; a click on
    /// one lists what it counts.
    pub(super) fn render_lens_layer(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let notes = self.lens_notes(ix);
        if notes.is_empty() {
            return None;
        }
        let doc = self.docs.get(ix)?;
        let editor = doc.editor.clone();
        let path = doc.path.clone();
        let workspace = cx.entity().downgrade();
        let theme = cx.theme();
        let color = theme.muted_foreground.opacity(0.8);
        let hover = theme.link;
        let font = Font {
            family: theme.mono_font_family.clone(),
            ..Default::default()
        };
        let font_size = theme.mono_font_size;
        let shape_font = font.clone();
        Some(
            canvas(
                move |bounds, window, cx| {
                    let state = editor.read(cx);
                    let mut placed = Vec::new();
                    // Rows off screen would be placed on the first one shown.
                    let visible = state.visible_row_range().unwrap_or_default();
                    for (row, col, note) in &notes {
                        if !visible.contains(&(*row as usize)) {
                            continue;
                        }
                        let end = state.text().line_end_offset(*row as usize);
                        let Some(at) = state.range_to_bounds(&(end..end)) else {
                            continue;
                        };
                        if at.bottom() < bounds.top() || at.top() > bounds.bottom() {
                            continue;
                        }
                        let line_height = at.size.height;
                        let text_size = font_size.min(line_height * 0.8) * 0.9;
                        let run = TextRun {
                            len: note.len(),
                            font: shape_font.clone(),
                            color,
                            background_color: None,
                            underline: None,
                            strikethrough: None,
                        };
                        let line = window.text_system().shape_line(note.clone(), text_size, &[run], None);
                        let origin = point(at.origin.x + text_size * GAP, at.origin.y);
                        let area = Bounds::new(origin, size(line.width, line_height));
                        let hitbox = window.insert_hitbox(area, HitboxBehavior::BlockMouse);
                        placed.push((note.clone(), text_size, origin, line_height, hitbox, *row, *col));
                    }
                    placed
                },
                move |bounds, placed, window, cx| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for (note, text_size, origin, line_height, hitbox, row, col) in placed {
                            let hovered = hitbox.is_hovered(window);
                            let run = TextRun {
                                len: note.len(),
                                font: font.clone(),
                                color: if hovered { hover } else { color },
                                background_color: None,
                                underline: hovered.then(|| UnderlineStyle {
                                    thickness: px(1.),
                                    color: Some(hover),
                                    wavy: false,
                                }),
                                strikethrough: None,
                            };
                            let line = window.text_system().shape_line(note, text_size, &[run], None);
                            let _ = line.paint(origin, line_height, TextAlign::Left, None, window, cx);
                            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
                            let workspace = workspace.clone();
                            let path = path.clone();
                            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                                if phase != DispatchPhase::Bubble
                                    || event.button != MouseButton::Left
                                    || !hitbox.is_hovered(window)
                                {
                                    return;
                                }
                                cx.stop_propagation();
                                let path = path.clone();
                                let workspace = workspace.clone();
                                // The editor under the note is still handling the press.
                                window.defer(cx, move |window, cx| {
                                    workspace
                                        .update(cx, |this, cx| this.list_usages_at(path, row, col, window, cx))
                                        .ok();
                                });
                            });
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
