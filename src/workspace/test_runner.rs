//! Running Go tests from the editor: a run marker beside each test function
//! in `_test.go` files, and Ctrl+Shift+F10 for the test at the caret or the
//! package's tests. Output goes to the run console.

use std::path::{Path, PathBuf};

use gpui_kit::component::{RopeExt as _, Sizable as _};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::*;

use super::Workspace;
use super::layout::BottomTab;
use super::run_panel::Phase;
use crate::goindex::{self, Kind};
use crate::run;
use crate::ui::icon;

actions!(gust, [RunTestAtCaret]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-shift-f10", RunTestAtCaret, Some("Workspace"))]);
}

/// The kind of test a function name declares, if any.
fn test_kind(name: &str) -> Option<&'static str> {
    ["Test", "Benchmark", "Example", "Fuzz"].into_iter().find(|prefix| {
        name.strip_prefix(prefix)
            .is_some_and(|rest| rest.chars().next().is_none_or(|ch| !ch.is_lowercase()))
    })
}

/// What `go test` runs: one function of a package, or all of its tests.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TestTarget {
    pub dir: PathBuf,
    pub name: Option<String>,
}

/// The arguments of `go test` for `name` in the package `target`.
fn test_args(name: Option<&str>, target: &str) -> Vec<String> {
    let mut args = vec!["test".to_string(), "-v".to_string(), "-count=1".to_string()];
    match name {
        Some(name) if test_kind(name) == Some("Benchmark") => {
            args.extend(["-run".into(), "^$".into(), "-bench".into(), format!("^{name}$")]);
        }
        Some(name) => args.extend(["-run".into(), format!("^{name}$")]),
        None => {}
    }
    args.push(target.to_string());
    args
}

impl Workspace {
    /// The test functions of the open file at `ix`: their lines and names.
    fn test_functions(&self, ix: usize) -> Vec<(u32, String)> {
        let Some(doc) = self.docs.get(ix) else {
            return Vec::new();
        };
        if !doc.path.to_string_lossy().ends_with("_test.go") {
            return Vec::new();
        }
        doc.outline
            .iter()
            .filter(|item| item.kind == Kind::Func && test_kind(&item.name).is_some())
            .map(|item| (item.row, item.name.clone()))
            .collect()
    }

    /// Run `go test` for `target` in the run console.
    pub(super) fn run_test(&mut self, target: TestTarget, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_run(cx);
        self.save_all(cx);
        let root = self.root.clone();
        let relative = super::relative_path(&root, &target.dir);
        let (module, package) = run::module_for(&root, if relative.is_empty() { "." } else { &relative })
            .unwrap_or_else(|| (target.dir.clone(), ".".to_string()));
        let command = run::Command {
            program: "go".to_string(),
            args: test_args(target.name.as_deref(), &package),
            dir: module,
            // Tests see the environment of the selected run configuration.
            env: self
                .settings
                .runs
                .get(self.settings.selected_run)
                .map(|config| run::parse_env(&config.env))
                .unwrap_or_default(),
        };
        let console = self.console(window, cx);
        let started = console.update(cx, |console, cx| {
            console.print("\x1b[2J\x1b[3J\x1b[H", cx);
            console.print(&format!("\x1b[90m{}\x1b[0m\n", command.display_in(&root)), cx);
            console.start(&command, cx)
        });
        self.run.title = target
            .name
            .clone()
            .unwrap_or_else(|| format!("{} 的测试", if relative.is_empty() { "." } else { &relative }))
            .into();
        self.run.base = target.dir.clone();
        self.run.last_test = Some(target);
        self.run.phase = match started {
            Ok(()) => Phase::Running,
            Err(err) => {
                console.update(cx, |console, cx| {
                    console.print(&format!("\x1b[31m无法启动：{err}\x1b[0m\n"), cx)
                });
                Phase::Failed
            }
        };
        self.terminal_visible = true;
        self.bottom_tab = BottomTab::Run;
        cx.notify();
    }

    /// Run `go args…` in the module that holds `dir`, in the run console.
    pub(super) fn run_go_tool(
        &mut self,
        dir: &Path,
        args: &[&str],
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_run(cx);
        self.save_all(cx);
        let root = self.root.clone();
        let relative = super::relative_path(&root, dir);
        let module = run::module_for(&root, if relative.is_empty() { "." } else { &relative })
            .map_or_else(|| dir.to_path_buf(), |(module, _)| module);
        let command = run::Command {
            program: "go".to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            dir: module.clone(),
            env: Default::default(),
        };
        let console = self.console(window, cx);
        let started = console.update(cx, |console, cx| {
            console.print("\x1b[2J\x1b[3J\x1b[H", cx);
            console.print(&format!("\x1b[90m{}\x1b[0m\n", command.display_in(&root)), cx);
            console.start(&command, cx)
        });
        self.run.title = title.to_string().into();
        self.run.base = module;
        self.run.last_test = None;
        self.run.phase = match started {
            Ok(()) => Phase::Running,
            Err(_) => Phase::Failed,
        };
        self.terminal_visible = true;
        self.bottom_tab = BottomTab::Run;
        cx.notify();
    }

    pub(super) fn on_run_test_at_caret(&mut self, _: &RunTestAtCaret, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.active else {
            return;
        };
        let doc = &self.docs[ix];
        if doc.language != "go" {
            return;
        }
        let Some(dir) = doc.path.parent().map(Path::to_path_buf) else {
            return;
        };
        let row = doc.editor.read(cx).cursor_position().line;
        let name = goindex::enclosing(&doc.outline, row)
            .filter(|item| item.kind == Kind::Func && test_kind(&item.name).is_some())
            .filter(|_| doc.path.to_string_lossy().ends_with("_test.go"))
            .map(|item| item.name.clone());
        self.run_test(TestTarget { dir, name }, window, cx);
    }

    /// Run markers beside the test functions of the open file at `ix`.
    pub(super) fn render_test_markers(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tests = self.test_functions(ix);
        if tests.is_empty() {
            return None;
        }
        let doc = &self.docs[ix];
        let dir = doc.path.parent()?.to_path_buf();
        let editor = doc.editor.read(cx);
        let input = editor.input_bounds();
        let height = editor.line_height().unwrap_or(px(20.));
        let color = crate::theme::palette(cx).green;
        let markers: Vec<AnyElement> = tests
            .into_iter()
            .filter_map(|(row, name)| {
                let start = editor.text().line_start_offset(row as usize);
                let line = editor.range_to_bounds(&(start..start))?;
                if line.top() < input.top() || line.bottom() > input.bottom() {
                    return None;
                }
                let size = px(16.);
                let left = (line.left() - input.left() - px(20.)).max(px(2.));
                let top = line.top() - input.top() + (height - size) / 2.;
                let target = TestTarget {
                    dir: dir.clone(),
                    name: Some(name.clone()),
                };
                let tip: SharedString = format!("运行 {name}").into();
                Some(
                    div()
                        .id(SharedString::from(format!("run-test-{name}")))
                        .absolute()
                        .left(left)
                        .top(top)
                        .size(size)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .text_color(color)
                        .hover(|this| this.bg(color.opacity(0.18)))
                        .child(icon("play").xsmall())
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run_test(target.clone(), window, cx)
                        }))
                        .into_any_element(),
                )
            })
            .collect();
        Some(div().absolute().inset_0().children(markers).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{test_args, test_kind};

    #[test]
    fn test_functions_are_told_by_their_names() {
        assert_eq!(test_kind("TestLoad"), Some("Test"));
        assert_eq!(test_kind("Test"), Some("Test"));
        assert_eq!(test_kind("Test_load"), Some("Test"));
        assert_eq!(test_kind("Testify"), None);
        assert_eq!(test_kind("BenchmarkLoad"), Some("Benchmark"));
        assert_eq!(test_kind("helper"), None);
    }

    #[test]
    fn go_test_runs_one_function_or_the_package() {
        assert_eq!(test_args(Some("TestLoad"), "./store").join(" "), "test -v -count=1 -run ^TestLoad$ ./store");
        assert_eq!(
            test_args(Some("BenchmarkLoad"), ".").join(" "),
            "test -v -count=1 -run ^$ -bench ^BenchmarkLoad$ ."
        );
        assert_eq!(test_args(None, ".").join(" "), "test -v -count=1 .");
    }
}
