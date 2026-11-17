//! Building and running the project: run configurations, the run buttons in
//! the title bar, and the console in the bottom panel that the build and the
//! program write to.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use super::layout::BottomTab;
use crate::proc;
use crate::run::{self, RunConfig};
use crate::terminal::{TerminalEvent, TerminalView};
use crate::ui::{UI_SMALL, UI_TEXT, icon};

actions!(
    gust,
    [RunProgram, BuildProgram, StopProgram, EditRunConfigs, CloseRunConfigs]
);

/// GoLand's keys: Shift+F10 runs, Ctrl+F9 builds, Ctrl+F2 stops.
pub(super) fn init(cx: &mut App) {
    let workspace = Some("Workspace");
    cx.bind_keys([
        KeyBinding::new("shift-f10", RunProgram, workspace),
        KeyBinding::new("ctrl-f9", BuildProgram, workspace),
        KeyBinding::new("ctrl-f2", StopProgram, workspace),
        KeyBinding::new("escape", CloseRunConfigs, Some("RunConfigs")),
    ]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Idle,
    Building,
    Running,
    /// The build failed, or the program could not start.
    Failed,
    /// The program ended with this code, when it is known.
    Exited(Option<i32>),
    Stopped,
    /// A build on its own finished.
    Built,
}

pub(super) struct RunState {
    pub console: Option<Entity<TerminalView>>,
    pub phase: Phase,
    /// The configuration last started, for the console's header.
    pub title: SharedString,
    build: Task<()>,
    cancel: Arc<AtomicBool>,
    /// The folder the program ran in: output paths are read from there.
    pub(super) base: PathBuf,
    /// The tests run last, when the last run was of tests.
    pub(super) last_test: Option<super::test_runner::TestTarget>,
    pub editor: Option<ConfigEditor>,
    _subscription: Option<Subscription>,
}

impl RunState {
    pub fn new() -> Self {
        Self {
            console: None,
            phase: Phase::Idle,
            title: SharedString::default(),
            build: Task::ready(()),
            cancel: Arc::new(AtomicBool::new(false)),
            base: PathBuf::new(),
            last_test: None,
            editor: None,
            _subscription: None,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.phase, Phase::Building | Phase::Running)
    }
}

/// The configurations being edited, apart from the saved ones until applied.
pub(super) struct ConfigEditor {
    pub configs: Vec<RunConfig>,
    pub selected: usize,
    name: Entity<InputState>,
    package: Entity<InputState>,
    dir: Entity<InputState>,
    args: Entity<InputState>,
    build_flags: Entity<InputState>,
    output: Entity<InputState>,
    env: Entity<TextareaState>,
    focus: FocusHandle,
    /// The project root, which paths in the form are read from.
    root: PathBuf,
    /// Where the program runs when the form leaves the folder empty.
    default_dir: SharedString,
    _package_changed: Option<Subscription>,
}

impl ConfigEditor {
    fn fields(&self) -> [(&'static str, &Entity<InputState>, String); 6] {
        [
            ("名称", &self.name, String::new()),
            (
                "包路径",
                &self.package,
                "相对于项目根目录".to_string(),
            ),
            (
                "工作目录",
                &self.dir,
                format!("默认：{}", self.default_dir),
            ),
            ("程序参数", &self.args, String::new()),
            ("构建参数", &self.build_flags, "传给 go build".to_string()),
            ("输出文件", &self.output, "默认：临时目录".to_string()),
        ]
    }

    /// Work out again where the program runs by default, for the package typed.
    fn refresh_default_dir(&mut self, cx: &App) {
        let config = RunConfig {
            package: self.package.read(cx).value().to_string(),
            dir: String::new(),
            ..Default::default()
        };
        self.default_dir = config.work_dir(&self.root).to_string_lossy().into_owned().into();
    }

    /// The form's values, written into the selected configuration.
    fn store(&mut self, cx: &App) {
        let value = |input: &Entity<InputState>| input.read(cx).value().trim().to_string();
        let config = RunConfig {
            name: value(&self.name),
            package: value(&self.package),
            dir: value(&self.dir),
            args: value(&self.args),
            build_flags: value(&self.build_flags),
            output: value(&self.output),
            env: self.env.read(cx).value().to_string(),
        };
        if let Some(slot) = self.configs.get_mut(self.selected) {
            *slot = config;
        }
    }

    /// Show the configuration at `ix` in the form.
    fn load(&mut self, ix: usize, window: &mut Window, cx: &mut App) {
        self.selected = ix;
        let config = self.configs.get(ix).cloned().unwrap_or_default();
        let set = |input: &Entity<InputState>, value: String, window: &mut Window, cx: &mut App| {
            input.update(cx, |input, cx| input.set_value(value, window, cx));
        };
        set(&self.name, config.name, window, cx);
        set(&self.package, config.package, window, cx);
        set(&self.dir, config.dir, window, cx);
        set(&self.args, config.args, window, cx);
        set(&self.build_flags, config.build_flags, window, cx);
        set(&self.output, config.output, window, cx);
        self.env
            .update(cx, |input, cx| input.set_value(config.env, window, cx));
    }
}

/// What a build came to.
enum Built {
    Ok(String),
    Failed(String),
    Cancelled,
}

/// Run `command` to its end and collect what it wrote; `cancel` kills it.
fn build(command: &run::Command, cancel: &AtomicBool) -> Built {
    let mut process = proc::command(&command.program);
    process
        .args(&command.args)
        .current_dir(&command.dir)
        .envs(&command.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(err) => return Built::Failed(format!("无法启动 {}：{err}\n", command.program)),
    };
    fn drain(pipe: Option<impl std::io::Read + Send + 'static>) -> std::thread::JoinHandle<String> {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            String::from_utf8_lossy(&bytes).into_owned()
        })
    }
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let success = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Built::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => std::thread::sleep(Duration::from_millis(30)),
            Err(err) => return Built::Failed(format!("{err}\n")),
        }
    };
    let mut output = stdout.join().unwrap_or_default();
    output.push_str(&stderr.join().unwrap_or_default());
    if success {
        Built::Ok(output)
    } else {
        Built::Failed(output)
    }
}

const DIM: &str = "\x1b[90m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const RESET: &str = "\x1b[0m";

impl Workspace {
    fn run_configs(&self) -> &[RunConfig] {
        &self.settings.runs
    }

    fn selected_run(&self) -> Option<&RunConfig> {
        let configs = self.run_configs();
        configs.get(self.settings.selected_run.min(configs.len().saturating_sub(1)))
    }

    pub(super) fn on_run_program(&mut self, _: &RunProgram, window: &mut Window, cx: &mut Context<Self>) {
        self.start_run(false, window, cx);
    }

    pub(super) fn on_build_program(
        &mut self,
        _: &BuildProgram,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_run(true, window, cx);
    }

    pub(super) fn on_stop_program(&mut self, _: &StopProgram, _: &mut Window, cx: &mut Context<Self>) {
        self.stop_run(cx);
    }

    pub(super) fn on_edit_run_configs(
        &mut self,
        _: &EditRunConfigs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_run_configs(false, window, cx);
    }

    pub(super) fn on_close_run_configs(
        &mut self,
        _: &CloseRunConfigs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_run_configs(window, cx);
    }

    /// The console, made the first time something runs.
    pub(super) fn console(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<TerminalView> {
        if let Some(console) = &self.run.console {
            return console.clone();
        }
        let console = cx.new(TerminalView::console);
        let subscription = cx.subscribe_in(
            &console,
            window,
            |this, _, event: &TerminalEvent, window, cx| match event {
                TerminalEvent::Exited(code) => this.program_exited(*code, cx),
                TerminalEvent::OpenFile(path, row, col) => {
                    let base = this.run.base.clone();
                    this.open_reference(path, *row, *col, Some(&base), window, cx)
                }
            },
        );
        self.run.console = Some(console.clone());
        self.run._subscription = Some(subscription);
        console
    }

    /// Build the selected configuration, then run it unless `build_only`.
    pub(super) fn start_run(&mut self, build_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.selected_run().cloned() else {
            self.open_run_configs(true, window, cx);
            return;
        };
        self.stop_run(cx);
        // The build reads files from disk, not from the editor.
        self.save_all(cx);

        let root = self.root.clone();
        let command = config.build(&root);
        let console = self.console(window, cx);
        console.update(cx, |console, cx| {
            // A fresh screen and history for each run.
            console.print("\x1b[2J\x1b[3J\x1b[H", cx);
            console.print(&format!("{DIM}{}{RESET}\n", command.display_in(&root)), cx);
        });
        self.run.title = config.name.clone().into();
        self.run.base = config.run(&root).dir;
        self.run.last_test = None;
        self.run.phase = Phase::Building;
        self.terminal_visible = true;
        self.bottom_tab = BottomTab::Run;
        let cancel = Arc::new(AtomicBool::new(false));
        self.run.cancel = cancel.clone();
        cx.notify();

        self.run.build = cx.spawn_in(window, async move |this, cx| {
            let built = cx
                .background_executor()
                .spawn(async move { build(&command, &cancel) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.build_finished(built, &config, build_only, window, cx)
            })
            .ok();
        });
    }

    fn build_finished(
        &mut self,
        built: Built,
        config: &RunConfig,
        build_only: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(console) = self.run.console.clone() else {
            return;
        };
        let root = self.root.clone();
        match built {
            Built::Cancelled => {}
            Built::Failed(output) => {
                self.run.phase = Phase::Failed;
                console.update(cx, |console, cx| {
                    console.print(&output, cx);
                    console.print(&format!("\n{RED}编译失败{RESET}\n"), cx);
                });
            }
            Built::Ok(output) if build_only => {
                self.run.phase = Phase::Built;
                let executable = config.executable(&root);
                console.update(cx, |console, cx| {
                    console.print(&output, cx);
                    console.print(
                        &format!("{GREEN}编译成功{RESET} {DIM}→ {}{RESET}\n", executable.display()),
                        cx,
                    );
                });
            }
            Built::Ok(output) => {
                let command = config.run(&root);
                let started = console.update(cx, |console, cx| {
                    console.print(&output, cx);
                    console.print(&format!("{DIM}{}{RESET}\n", command.display_in(&root)), cx);
                    console.start(&command, cx)
                });
                match started {
                    Ok(()) => self.run.phase = Phase::Running,
                    Err(err) => {
                        self.run.phase = Phase::Failed;
                        console.update(cx, |console, cx| {
                            console.print(&format!("{RED}无法启动：{err}{RESET}\n"), cx)
                        });
                    }
                }
            }
        }
        cx.notify();
    }

    fn program_exited(&mut self, code: Option<i32>, cx: &mut Context<Self>) {
        if self.run.phase != Phase::Running {
            return;
        }
        self.run.phase = Phase::Exited(code);
        let tests = self.run.last_test.is_some();
        if let Some(console) = &self.run.console {
            let line = match (tests, code) {
                (true, Some(0)) => format!("\n{GREEN}测试通过{RESET}\n"),
                (true, _) => format!("\n{RED}测试失败{RESET}\n"),
                (false, code) => {
                    let code = code.map_or("未知".to_string(), |code| code.to_string());
                    format!("\n{DIM}进程已结束，退出代码 {code}{RESET}\n")
                }
            };
            console.update(cx, |console, cx| console.print(&line, cx));
        }
        cx.notify();
    }

    pub(super) fn stop_run(&mut self, cx: &mut Context<Self>) {
        if !self.run.busy() {
            return;
        }
        self.run.cancel.store(true, Ordering::Relaxed);
        self.run.phase = Phase::Stopped;
        if let Some(console) = &self.run.console {
            console.update(cx, |console, cx| {
                console.stop();
                console.print(&format!("\n{DIM}已停止{RESET}\n"), cx);
            });
        }
        cx.notify();
    }

    /// Open the file a `file.go:12:5` in output names: as written when it is
    /// absolute, otherwise from `base` or from the project root.
    pub(super) fn open_reference(
        &mut self,
        path: &str,
        row: u32,
        col: u32,
        base: Option<&Path>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let written = Path::new(path);
        let candidates = [
            written.is_absolute().then(|| written.to_path_buf()),
            base.map(|base| base.join(written)),
            Some(self.root.join(written)),
        ];
        if let Some(file) = candidates.into_iter().flatten().find(|file| file.is_file()) {
            self.open_at(file, row, col, window, cx);
        }
    }

    // ------------------------------------------------------------ configurations

    /// The configuration editor; `add` starts a new configuration in it.
    pub(super) fn open_run_configs(&mut self, add: bool, window: &mut Window, cx: &mut Context<Self>) {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
        };
        let mut editor = ConfigEditor {
            configs: self.settings.runs.clone(),
            selected: self.settings.selected_run,
            name: input("", window, cx),
            package: input(".", window, cx),
            dir: input("", window, cx),
            args: input("", window, cx),
            build_flags: input("", window, cx),
            output: input("", window, cx),
            env: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .placeholder("KEY=VALUE，每行一个")
                    .rows(4)
            }),
            focus: cx.focus_handle(),
            root: self.root.clone(),
            default_dir: SharedString::default(),
            _package_changed: None,
        };
        // The default folder follows the package as it is typed.
        editor._package_changed = Some(cx.subscribe(&editor.package, |this, _, event: &gpui_kit::component::input::InputEvent, cx| {
            if matches!(event, gpui_kit::component::input::InputEvent::Change)
                && let Some(editor) = &mut this.run.editor
            {
                editor.refresh_default_dir(cx);
                cx.notify();
            }
        }));
        if add || editor.configs.is_empty() {
            editor.configs.push(self.new_run_config(&editor.configs));
            editor.selected = editor.configs.len() - 1;
        }
        let selected = editor.selected.min(editor.configs.len() - 1);
        editor.load(selected, window, cx);
        editor.refresh_default_dir(cx);
        editor.name.update(cx, |input, cx| input.focus(window, cx));
        self.run.editor = Some(editor);
        cx.notify();
    }

    /// A new configuration: the package of the open Go file with a `main`, or the root.
    fn new_run_config(&self, existing: &[RunConfig]) -> RunConfig {
        let package = self
            .active
            .and_then(|ix| self.docs.get(ix))
            .filter(|doc| doc.language == "go")
            .filter(|doc| {
                doc.saved.contains("package main") && doc.saved.contains("func main()")
            })
            .and_then(|doc| doc.path.parent())
            .and_then(|dir| dir.strip_prefix(&self.root).ok())
            .map(|dir| dir.to_string_lossy().replace('\\', "/"))
            .filter(|dir| !dir.is_empty())
            .map(|dir| format!("./{dir}"))
            // Otherwise the module with a `main` package, wherever it sits.
            .or_else(|| {
                let (module, _) = run::module_for(&self.root, ".")?;
                let inside = module.strip_prefix(&self.root).ok()?;
                (!inside.as_os_str().is_empty())
                    .then(|| format!("./{}", inside.to_string_lossy().replace('\\', "/")))
            })
            .unwrap_or_else(|| ".".to_string());
        let base = package
            .rsplit('/')
            .next()
            .filter(|name| *name != ".")
            .map(str::to_string)
            .unwrap_or_else(|| self.project_name().to_string());
        let mut name = base.clone();
        let mut n = 2;
        while existing.iter().any(|config| config.name == name) {
            name = format!("{base} {n}");
            n += 1;
        }
        RunConfig {
            name,
            package,
            ..Default::default()
        }
    }

    pub(super) fn close_run_configs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.run.editor.take().is_some() {
            self.focus_editor(window, cx);
            cx.notify();
        }
    }

    /// Keep the edited configurations; with `run`, start the selected one too.
    pub(super) fn apply_run_configs(&mut self, run: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut editor) = self.run.editor.take() else {
            return;
        };
        editor.store(cx);
        for (ix, config) in editor.configs.iter_mut().enumerate() {
            if config.name.is_empty() {
                config.name = format!("配置 {}", ix + 1);
            }
        }
        self.settings.runs = editor.configs;
        self.settings.selected_run = editor.selected;
        if let Err(reason) = crate::settings::save(&self.root, &self.settings) {
            self.notice = Some(reason.into());
        }
        self.focus_editor(window, cx);
        if run && !self.settings.runs.is_empty() {
            self.start_run(false, window, cx);
        }
        cx.notify();
    }

    fn select_run_config(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.run.editor else {
            return;
        };
        editor.store(cx);
        editor.load(ix, window, cx);
        cx.notify();
    }

    fn add_run_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let config = match &self.run.editor {
            Some(editor) => self.new_run_config(&editor.configs),
            None => return,
        };
        let Some(editor) = &mut self.run.editor else {
            return;
        };
        editor.store(cx);
        editor.configs.push(config);
        let last = editor.configs.len() - 1;
        editor.load(last, window, cx);
        editor.name.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn remove_run_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.run.editor else {
            return;
        };
        if editor.selected < editor.configs.len() {
            editor.configs.remove(editor.selected);
        }
        let selected = editor.selected.min(editor.configs.len().saturating_sub(1));
        editor.load(selected, window, cx);
        cx.notify();
    }

    fn choose_run_config(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.settings.selected_run = ix;
        if let Err(reason) = crate::settings::save(&self.root, &self.settings) {
            self.notice = Some(reason.into());
        }
        cx.notify();
    }

    // ------------------------------------------------------------ rendering

    /// The configuration menu and the run, build, and stop buttons.
    pub(super) fn render_run_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
        use gpui_kit::component::tooltip::Tooltip;

        let theme = cx.theme();
        let workspace = cx.entity().downgrade();
        let names: Vec<String> = self.run_configs().iter().map(|c| c.name.clone()).collect();
        let selected = self.settings.selected_run.min(names.len().saturating_sub(1));
        let busy = self.run.busy();

        let chooser = if names.is_empty() {
            Button::new("run-configs")
                .ghost()
                .small()
                .icon(icon("circle-plus"))
                .text_color(theme.muted_foreground)
                .label("添加运行配置…")
                .on_click(cx.listener(|this, _, window, cx| this.open_run_configs(true, window, cx)))
                .into_any_element()
        } else {
            Button::new("run-configs")
                .ghost()
                .small()
                .label(names[selected].clone())
                .dropdown_caret(true)
                .dropdown_menu(move |menu, window, _| {
                    let mut menu = Self::long_menu(menu, window);
                    for (ix, name) in names.iter().enumerate() {
                        let choose = workspace.clone();
                        menu = menu.item(
                            PopupMenuItem::new(name.clone())
                                .checked(ix == selected)
                                .on_click(move |_, _, cx| {
                                    choose.update(cx, |this, cx| this.choose_run_config(ix, cx)).ok();
                                }),
                        );
                    }
                    let edit = workspace.clone();
                    menu.separator().item(
                        PopupMenuItem::new("编辑配置…")
                            .icon(icon("settings"))
                            .on_click(move |_, window, cx| {
                                edit.update(cx, |this, cx| this.open_run_configs(false, window, cx))
                                    .ok();
                            }),
                    )
                })
                .into_any_element()
        };

        let button = |id: &'static str, name: &'static str, color: Hsla, tip: SharedString| {
            div()
                .id(id)
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .text_color(color)
                .hover(|this| this.bg(theme.list_hover))
                .child(icon(name).small())
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
        };
        let colors = crate::theme::palette(cx);
        let name = self.selected_run().map(|c| c.name.clone()).unwrap_or_default();
        h_flex()
            .gap_0p5()
            .child(chooser)
            .child(
                button(
                    "run-start",
                    if busy { "rotate-cw" } else { "play" },
                    colors.green,
                    self.tip(&format!("{} '{name}'", if busy { "重新运行" } else { "运行" }), "run"),
                )
                .on_click(cx.listener(|this, _, window, cx| this.start_run(false, window, cx))),
            )
            .child(
                button("run-build", "hammer", theme.muted_foreground, self.tip("编译", "build"))
                    .on_click(cx.listener(|this, _, window, cx| this.start_run(true, window, cx))),
            )
            .when(busy, |bar| {
                bar.child(
                    button("run-stop", "square", colors.red, self.tip("停止", "stop"))
                        .on_click(cx.listener(|this, _, _, cx| this.stop_run(cx))),
                )
            })
    }

    /// The status of the last run, for the bottom panel's header.
    pub(super) fn render_run_status(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let colors = crate::theme::palette(cx);
        let muted = cx.theme().muted_foreground;
        let (text, color) = match self.run.phase {
            Phase::Idle => (String::new(), muted),
            Phase::Building => ("正在编译…".to_string(), muted),
            Phase::Running => ("运行中".to_string(), colors.green),
            Phase::Failed => ("失败".to_string(), colors.red),
            Phase::Built => ("编译成功".to_string(), colors.green),
            Phase::Stopped => ("已停止".to_string(), muted),
            Phase::Exited(Some(0)) => ("已结束".to_string(), muted),
            Phase::Exited(Some(code)) => (format!("退出代码 {code}"), colors.red),
            Phase::Exited(None) => ("已结束".to_string(), muted),
        };
        let busy = self.run.busy();
        h_flex()
            .gap_1()
            .child(
                div()
                    .text_size(UI_SMALL)
                    .text_color(color)
                    .mr_1()
                    .child(text),
            )
            .child(
                Self::header_button("console-rerun", "rotate-cw", "重新运行", cx).on_click(
                    cx.listener(|this, _, window, cx| match this.run.last_test.clone() {
                        Some(target) => this.run_test(target, window, cx),
                        None => this.start_run(false, window, cx),
                    }),
                ),
            )
            .when(busy, |bar| {
                bar.child(
                    Self::header_button("console-stop", "square", "停止 (Ctrl+F2)", cx)
                        .text_color(colors.red)
                        .on_click(cx.listener(|this, _, _, cx| this.stop_run(cx))),
                )
            })
    }

    /// The configuration editor as a layer over the window.
    pub(super) fn render_run_configs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        use gpui_kit::component::button::{Button, ButtonVariants as _};

        let editor = self.run.editor.as_ref()?;
        let theme = cx.theme();
        let list = editor.configs.iter().enumerate().map(|(ix, config)| {
            let name = if ix == editor.selected {
                editor.name.read(cx).value().to_string()
            } else {
                config.name.clone()
            };
            h_flex()
                .id(("run-config", ix))
                .h(px(28.))
                .px_2()
                .gap_2()
                .rounded(px(4.))
                .text_size(UI_TEXT)
                .when(ix == editor.selected, |row| row.bg(theme.list_active))
                .hover(|row| row.bg(theme.list_hover))
                .child(icon("play").xsmall().text_color(theme.muted_foreground))
                .child(div().truncate().child(if name.is_empty() { "未命名".to_string() } else { name }))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_run_config(ix, window, cx)
                }))
        });
        let has_config = !editor.configs.is_empty();
        let label = |text: &'static str| {
            div()
                .w(px(76.))
                .flex_shrink_0()
                .pt(px(5.))
                .text_size(UI_SMALL)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let form = v_flex()
            .flex_1()
            .min_w_0()
            .p_4()
            .gap_2()
            .children(editor.fields().into_iter().map(|(name, input, hint)| {
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(label(name))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(Input::new(input).small())
                            .when(!hint.is_empty(), |field| {
                                field.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(theme.muted_foreground)
                                        .child(hint),
                                )
                            }),
                    )
            }))
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(label("环境变量"))
                    .child(div().flex_1().min_w_0().child(Textarea::new(&editor.env).small())),
            );

        let panel = v_flex()
            .id("run-configs")
            .key_context("RunConfigs")
            .track_focus(&editor.focus)
            .occlude()
            .w(px(760.))
            .max_w_full()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                h_flex()
                    .h(px(40.))
                    .px_4()
                    .border_b_1()
                    .border_color(theme.border)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(UI_TEXT)
                    .child("运行配置"),
            )
            .child(
                h_flex()
                    .items_start()
                    .h(px(420.))
                    .child(
                        v_flex()
                            .w(px(200.))
                            .h_full()
                            .flex_shrink_0()
                            .border_r_1()
                            .border_color(theme.border)
                            .bg(theme.sidebar)
                            .child(
                                v_flex()
                                    .id("run-config-list")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .p_1()
                                    .children(list),
                            )
                            .child(
                                h_flex()
                                    .p_1()
                                    .gap_1()
                                    .border_t_1()
                                    .border_color(theme.border)
                                    .child(
                                        Self::header_button("run-config-add", "plus", "新建配置", cx)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.add_run_config(window, cx)
                                            })),
                                    )
                                    .when(has_config, |bar| {
                                        bar.child(
                                            Self::header_button(
                                                "run-config-remove",
                                                "minus",
                                                "删除配置",
                                                cx,
                                            )
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.remove_run_config(window, cx)
                                            })),
                                        )
                                    }),
                            ),
                    )
                    .when(has_config, |body| body.child(form))
                    .when(!has_config, |body| {
                        body.child(
                            div()
                                .flex_1()
                                .p_4()
                                .text_size(UI_TEXT)
                                .text_color(theme.muted_foreground)
                                .child("没有运行配置"),
                        )
                    }),
            )
            .child(
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .justify_end()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("run-config-cancel")
                            .small()
                            .label("取消")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_run_configs(window, cx)
                            })),
                    )
                    .child(
                        Button::new("run-config-save")
                            .small()
                            .label("保存")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.apply_run_configs(false, window, cx)
                            })),
                    )
                    .when(has_config, |footer| {
                        footer.child(
                            Button::new("run-config-run")
                                .small()
                                .primary()
                                .label("保存并运行")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.apply_run_configs(true, window, cx)
                                })),
                        )
                    }),
            );

        Some(
            div()
                .id("run-configs-layer")
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .items_start()
                .pt(px(64.))
                .bg(black().opacity(0.25))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.close_run_configs(window, cx)),
                )
                .child(panel)
                .into_any_element(),
        )
    }
}
