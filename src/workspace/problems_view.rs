//! Compile errors and vet findings in the editor: squiggles under them, a
//! count in the status bar that opens their list, and F2 / Shift+F2 to go
//! from one to the next, as in GoLand.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::base::input::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::RopeExt as _;
use gpui_kit::component::input::Position;
use gpui_kit::*;

use super::Workspace;
use super::palette::Mode;
use crate::finder::Hit;
use crate::problems::{self, Problem};

actions!(gust, [NextProblem, PreviousProblem, ShowProblems]);

pub(super) fn init(cx: &mut App) {
    let workspace = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("f2", NextProblem, workspace),
        KeyBinding::new("shift-f2", PreviousProblem, workspace),
        KeyBinding::new("alt-6", ShowProblems, workspace),
    ]);
}

/// What the last checks found, by package folder.
#[derive(Default)]
pub(super) struct Problems {
    pub by_package: HashMap<PathBuf, Vec<Problem>>,
    checks: HashMap<PathBuf, Task<()>>,
}

impl Problems {
    pub fn counts(&self) -> (usize, usize) {
        let all = self.by_package.values().flatten();
        let errors = all.clone().filter(|problem| problem.error).count();
        (errors, all.count() - errors)
    }

    fn of_file<'a>(&'a self, file: &'a Path) -> impl Iterator<Item = &'a Problem> + 'a {
        file.parent()
            .and_then(|dir| self.by_package.get(dir))
            .into_iter()
            .flatten()
            .filter(move |problem| problem.file == file)
    }
}

impl Workspace {
    /// Check the package of the Go file `path`, once things settle.
    pub(super) fn check_package(&mut self, path: &Path, cx: &mut Context<Self>) {
        if path.extension().is_none_or(|ext| ext != "go") {
            return;
        }
        let Some(dir) = path.parent().map(Path::to_path_buf) else {
            return;
        };
        let package = dir.clone();
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            let found = cx
                .background_executor()
                .spawn(async move { problems::check(&package) })
                .await;
            this.update(cx, |this, cx| {
                this.problems.by_package.insert(dir.clone(), found);
                for ix in 0..this.docs.len() {
                    if this.docs[ix].path.parent() == Some(dir.as_path()) {
                        this.show_problems_in(ix, cx);
                    }
                }
                cx.notify();
            })
            .ok();
        });
        self.problems.checks.insert(path.parent().unwrap().to_path_buf(), task);
    }

    /// Underline what the last check found in the open file at `ix`.
    pub(super) fn show_problems_in(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(doc) = self.docs.get(ix) else {
            return;
        };
        let found: Vec<Problem> = self.problems.of_file(&doc.path).cloned().collect();
        doc.editor.update(cx, |editor, cx| {
            let text = editor.text().clone();
            let Some(set) = editor.diagnostics_mut() else {
                return;
            };
            set.reset(&text);
            for problem in found {
                let line = text.slice_line(problem.row as usize).to_string();
                let line = line.trim_end();
                let mut start = (problem.col as usize).min(line.len());
                while !line.is_char_boundary(start) {
                    start -= 1;
                }
                // The word the problem points at, or one character.
                let word = line[start..]
                    .find(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                    .unwrap_or(line.len() - start);
                let end = if word > 0 {
                    start + word
                } else {
                    line[start..].chars().next().map_or(start, |ch| start + ch.len_utf8())
                };
                let column = |byte: usize| line[..byte].encode_utf16().count() as u32;
                set.push(Diagnostic {
                    range: Position::new(problem.row, column(start))
                        ..Position::new(problem.row, column(end)),
                    severity: if problem.error {
                        DiagnosticSeverity::Error
                    } else {
                        DiagnosticSeverity::Warning
                    },
                    source: Some("go".into()),
                    message: problem.message.into(),
                    ..Default::default()
                });
            }
            cx.notify();
        });
    }

    /// Go to the next problem in the open file after the caret, or the one before.
    fn step_problem(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.active.and_then(|ix| self.docs.get(ix)) else {
            return;
        };
        let here = doc.editor.read(cx).cursor_position();
        let mut found: Vec<(u32, u32)> = self
            .problems
            .of_file(&doc.path)
            .map(|problem| (problem.row, problem.col))
            .collect();
        found.sort();
        found.dedup();
        let caret = (here.line, here.character);
        let next = if forward {
            found.iter().find(|at| **at > caret).or(found.first())
        } else {
            found.iter().rev().find(|at| **at < caret).or(found.last())
        };
        if let Some(&(row, col)) = next {
            let path = doc.path.clone();
            self.open_at(path, row, col, window, cx);
        }
    }

    pub(super) fn on_next_problem(&mut self, _: &NextProblem, window: &mut Window, cx: &mut Context<Self>) {
        self.step_problem(true, window, cx);
    }

    pub(super) fn on_previous_problem(
        &mut self,
        _: &PreviousProblem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_problem(false, window, cx);
    }

    /// Every problem found, errors first, to pick one and go there.
    pub(super) fn on_show_problems(&mut self, _: &ShowProblems, window: &mut Window, cx: &mut Context<Self>) {
        let mut all: Vec<&Problem> = self.problems.by_package.values().flatten().collect();
        all.sort_by(|a, b| (!a.error, &a.file, a.row).cmp(&(!b.error, &b.file, b.row)));
        let hits: Vec<Hit> = all
            .into_iter()
            .map(|problem| Hit {
                path: super::relative_path(&self.root, &problem.file),
                row: problem.row,
                col: problem.col,
                preview: problem.message.clone(),
                tag: if problem.error { "错误" } else { "警告" },
                detail: String::new(),
            })
            .collect();
        let (errors, warnings) = self.problems.counts();
        self.open_palette(Mode::Problems, window, cx);
        if let Some(palette) = &mut self.palette {
            palette.title = Some(format!("{errors} 个错误 · {warnings} 个警告").into());
            palette.found = hits.clone();
            palette.hits = hits;
        }
        cx.notify();
    }
}
