//! A terminal panel: a shell on a pseudo-terminal, drawn as a grid of cells.
//!
//! `alacritty_terminal` owns the pty, the escape-sequence parser, and the
//! grid; this file turns keys into the bytes a terminal sends and paints the
//! grid with GPUI.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use alacritty_terminal::event::{Event as PtyEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Processor, Rgb};
use gpui_kit::component::{ActiveTheme as _, Theme};
use gpui_kit::*;

use crate::proc;
use crate::run;

/// Keys the workspace binds that a shell needs for itself while it has focus.
pub fn init(cx: &mut App) {
    let terminal = Some("Terminal");
    cx.bind_keys(
        ["ctrl-s", "ctrl-w", "ctrl-o", "ctrl-p", "ctrl-f", "ctrl-k"]
            .map(|key| KeyBinding::new(key, NoAction, terminal)),
    );
}

pub enum TerminalEvent {
    /// The process ended, with its exit code when it is known.
    Exited(Option<i32>),
    /// A `file.go:12:5` in the output was clicked: the path as written, and
    /// the zero-based row and byte column.
    OpenFile(String, u32, u32),
}

/// Lines a terminal keeps above the screen.
const SHELL_HISTORY: usize = 10_000;
/// A program's output is read once; a shorter history keeps a chatty
/// program from holding on to much memory.
const CONSOLE_HISTORY: usize = 5_000;

#[derive(Clone)]
struct Listener(async_channel::Sender<PtyEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: PtyEvent) {
        let _ = self.0.try_send(event);
    }
}

struct GridSize {
    cols: usize,
    lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.cols
    }
}

pub struct TerminalView {
    term: Arc<FairMutex<Term<Listener>>>,
    pty: Option<EventLoopSender>,
    focus: FocusHandle,
    cols: usize,
    lines: usize,
    /// Size of one cell and where the grid starts, as of the last frame.
    cell: Size<Pixels>,
    origin: Point<Pixels>,
    /// Wheel movement not yet large enough to scroll a whole line.
    scroll_remainder: Pixels,
    selecting: bool,
    /// Text an input method is still composing.
    marked: Option<String>,
    error: Option<SharedString>,
    /// Where the process sends its events; kept to start another one.
    listener: Listener,
    /// Parses text printed into the terminal by Gust itself.
    parser: Processor,
    /// The process has ended, and that has been told.
    exited: bool,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

fn default_shell() -> (String, Vec<String>) {
    if cfg!(windows) {
        for shell in ["pwsh", "powershell"] {
            if proc::on_path(shell) {
                return (format!("{shell}.exe"), vec!["-NoLogo".to_string()]);
            }
        }
        ("cmd.exe".to_string(), Vec::new())
    } else {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, Vec::new())
    }
}

impl TerminalView {
    /// A shell in `cwd`.
    pub fn new(cwd: PathBuf, cx: &mut Context<Self>) -> Self {
        let mut this = Self::empty(SHELL_HISTORY, cx);
        // Tests get a terminal without a shell: a real one runs on its own
        // threads, which would make them depend on timing.
        if !cfg!(test) {
            let (program, args) = default_shell();
            let env = HashMap::from([
                ("TERM".to_string(), "xterm-256color".to_string()),
                ("COLORTERM".to_string(), "truecolor".to_string()),
            ]);
            if let Err(err) = this.spawn(program, args, cwd, env, false) {
                this.error = Some(format!("无法启动终端：{err}").into());
            }
        }
        this
    }

    /// A terminal with nothing running yet, for a program's output.
    pub fn console(cx: &mut Context<Self>) -> Self {
        Self::empty(CONSOLE_HISTORY, cx)
    }

    /// Start `command` here; what is on screen stays above its output.
    pub fn start(&mut self, command: &run::Command, cx: &mut Context<Self>) -> Result<(), String> {
        self.stop();
        if cfg!(test) {
            return Ok(());
        }
        let started = self.spawn(
            command.program.clone(),
            command.args.clone(),
            command.dir.clone(),
            command.env.clone(),
            true,
        );
        cx.notify();
        started.map_err(|err| err.to_string())
    }

    /// End the process, if one is running.
    pub fn stop(&mut self) {
        if let Some(pty) = self.pty.take() {
            let _ = pty.send(Msg::Shutdown);
        }
    }

    /// Show `text` as if a program had written it; `\n` starts a new line.
    pub fn print(&mut self, text: &str, cx: &mut Context<Self>) {
        let bytes = text.replace("\r\n", "\n").replace('\n', "\r\n");
        let mut term = self.term.lock();
        term.scroll_display(Scroll::Bottom);
        self.parser.advance(&mut *term, bytes.as_bytes());
        drop(term);
        cx.notify();
    }

    /// Everything in the terminal, history included, one line per row.
    #[cfg(test)]
    pub fn text(&self) -> String {
        let term = self.term.lock();
        let grid = term.grid();
        let start = GridPoint::new(grid.topmost_line(), Column(0));
        let end = GridPoint::new(grid.bottommost_line(), grid.last_column());
        term.bounds_to_string(start, end)
            .lines()
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("\n")
            .trim_end()
            .to_string()
    }

    fn spawn(
        &mut self,
        program: String,
        args: Vec<String>,
        cwd: PathBuf,
        env: HashMap<String, String>,
        drain_on_exit: bool,
    ) -> std::io::Result<()> {
        let options = tty::Options {
            shell: Some(tty::Shell::new(program, args)),
            working_directory: Some(cwd),
            drain_on_exit,
            env,
            #[cfg(windows)]
            escape_args: true,
        };
        let pty = tty::new(&options, self.window_size(), 0)?;
        let event_loop = EventLoop::new(self.term.clone(), self.listener.clone(), pty, false, false)?;
        let sender = event_loop.channel();
        event_loop.spawn();
        self.pty = Some(sender);
        self.exited = false;
        Ok(())
    }

    fn empty(history: usize, cx: &mut Context<Self>) -> Self {
        let (cols, lines) = (80, 24);
        let (sender, receiver) = async_channel::unbounded();
        let listener = Listener(sender);
        let config = Config {
            scrolling_history: history,
            ..Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &GridSize { cols, lines },
            listener.clone(),
        )));

        cx.spawn(async move |this, cx| {
            while let Ok(event) = receiver.recv().await {
                // Output arrives in bursts; take the whole burst in one update.
                let mut events = vec![event];
                while let Ok(event) = receiver.try_recv() {
                    events.push(event);
                }
                let updated = this.update(cx, |this, cx| {
                    for event in events {
                        this.on_pty_event(event, cx);
                    }
                });
                if updated.is_err() {
                    break;
                }
            }
        })
        .detach();

        Self {
            term,
            pty: None,
            focus: cx.focus_handle(),
            cols,
            lines,
            cell: size(px(8.), px(16.)),
            origin: Point::default(),
            scroll_remainder: px(0.),
            selecting: false,
            marked: None,
            error: None,
            listener,
            parser: Processor::new(),
            exited: false,
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus.focus(window, cx);
    }

    fn on_pty_event(&mut self, event: PtyEvent, cx: &mut Context<Self>) {
        match event {
            PtyEvent::Wakeup | PtyEvent::CursorBlinkingChange | PtyEvent::MouseCursorDirty => {
                cx.notify()
            }
            PtyEvent::PtyWrite(text) => self.send(text.into_bytes()),
            PtyEvent::ClipboardStore(_, text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text))
            }
            PtyEvent::ClipboardLoad(_, format) => {
                let text = cx
                    .read_from_clipboard()
                    .and_then(|item| item.text())
                    .unwrap_or_default();
                self.send(format(&text).into_bytes());
            }
            PtyEvent::ColorRequest(index, format) => {
                let palette = Palette::new(cx.theme());
                let color = self.term.lock().colors()[index]
                    .unwrap_or_else(|| palette.rgb(index));
                self.send(format(color).into_bytes());
            }
            PtyEvent::TextAreaSizeRequest(format) => {
                let size = self.window_size();
                self.send(format(size).into_bytes());
            }
            PtyEvent::ChildExit(status) => {
                if !self.exited {
                    self.exited = true;
                    cx.emit(TerminalEvent::Exited(status.code()));
                }
            }
            PtyEvent::Exit => {
                if !self.exited {
                    self.exited = true;
                    cx.emit(TerminalEvent::Exited(None));
                }
            }
            PtyEvent::Title(_) | PtyEvent::ResetTitle | PtyEvent::Bell => {}
        }
    }

    fn window_size(&self) -> WindowSize {
        WindowSize {
            num_lines: self.lines as u16,
            num_cols: self.cols as u16,
            cell_width: f32::from(self.cell.width) as u16,
            cell_height: f32::from(self.cell.height) as u16,
        }
    }

    /// Bytes for the shell, without touching the view.
    fn send(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if let Some(pty) = &self.pty
            && !bytes.is_empty()
        {
            let _ = pty.send(Msg::Input(bytes));
        }
    }

    /// Bytes the user typed: the view follows the cursor back to the bottom.
    fn write(&mut self, bytes: impl Into<Cow<'static, [u8]>>, cx: &mut Context<Self>) {
        {
            let mut term = self.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.send(bytes);
        cx.notify();
    }

    /// Fit the grid to the space it is drawn in.
    fn set_size(&mut self, cols: usize, lines: usize, cell: Size<Pixels>, origin: Point<Pixels>) {
        self.cell = cell;
        self.origin = origin;
        if cols == self.cols && lines == self.lines {
            return;
        }
        self.cols = cols;
        self.lines = lines;
        self.term.lock().resize(GridSize { cols, lines });
        if let Some(pty) = &self.pty {
            let _ = pty.send(Msg::Resize(self.window_size()));
        }
    }

    fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    fn copy_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let text = self.term.lock().selection_to_string();
        match text.filter(|text| !text.is_empty()) {
            Some(text) => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                self.term.lock().selection = None;
                cx.notify();
                true
            }
            None => false,
        }
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) else {
            return;
        };
        let bytes = if self.mode().contains(TermMode::BRACKETED_PASTE) {
            // An escape in the pasted text could end the bracket early.
            format!("\x1b[200~{}\x1b[201~", text.replace('\x1b', ""))
        } else {
            text.replace("\r\n", "\r").replace('\n', "\r")
        };
        self.write(bytes.into_bytes(), cx);
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let handled = match keystroke.key.as_str() {
            "c" if modifiers.control && modifiers.shift => {
                self.copy_selection(cx);
                true
            }
            // Like Windows Terminal: Ctrl+C copies when there is a selection.
            "c" if modifiers.control && !modifiers.alt => {
                if !self.copy_selection(cx) {
                    self.write(vec![0x03], cx);
                }
                true
            }
            "v" if modifiers.control && !modifiers.alt => {
                self.paste(cx);
                true
            }
            "insert" if modifiers.shift => {
                self.paste(cx);
                true
            }
            _ => match key_bytes(keystroke, self.mode()) {
                Some(bytes) => {
                    self.write(bytes, cx);
                    true
                }
                None => false,
            },
        };
        // Anything else is text: it arrives through the input handler, which
        // is also how an input method delivers what it composed.
        if handled {
            cx.stop_propagation();
        }
    }

    fn grid_point(&self, position: Point<Pixels>) -> (GridPoint, Side) {
        let x = (position.x - self.origin.x).max(px(0.)) / self.cell.width;
        let y = (position.y - self.origin.y).max(px(0.)) / self.cell.height;
        let col = (x as usize).min(self.cols.saturating_sub(1));
        let row = (y as usize).min(self.lines.saturating_sub(1));
        let side = if x.fract() < 0.5 {
            Side::Left
        } else {
            Side::Right
        };
        let offset = self.term.lock().grid().display_offset() as i32;
        (GridPoint::new(Line(row as i32 - offset), Column(col)), side)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        let (point, side) = self.grid_point(event.position);
        let kind = match event.click_count {
            0 | 1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        self.term.lock().selection = Some(Selection::new(kind, point, side));
        self.selecting = true;
        cx.notify();
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let (point, side) = self.grid_point(event.position);
        if let Some(selection) = self.term.lock().selection.as_mut() {
            selection.update(point, side);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.selecting = false;
        let mut term = self.term.lock();
        // A click without a drag selects nothing.
        let clicked = term.selection.as_ref().is_some_and(Selection::is_empty);
        if clicked {
            term.selection = None;
        }
        drop(term);
        if clicked
            && event.click_count == 1
            && let Some((path, row, col)) = self.reference_at(event.position)
        {
            cx.emit(TerminalEvent::OpenFile(path, row, col));
        }
        cx.notify();
    }

    /// The `file.go:12` written where `position` is, if any.
    fn reference_at(&self, position: Point<Pixels>) -> Option<(String, u32, u32)> {
        let (point, _) = self.grid_point(position);
        let term = self.term.lock();
        let row = &term.grid()[point.line];
        // The line as text, and the byte each column starts at.
        let mut line = String::new();
        let mut starts = Vec::with_capacity(self.cols);
        for col in 0..self.cols.min(row.len()) {
            starts.push(line.len());
            let cell = &row[Column(col)];
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                line.push(cell.c);
            }
        }
        let at = *starts.get(point.column.0)?;
        run::file_reference_at(&line, at)
    }

    /// Right click copies the selection, or pastes when there is none.
    fn on_right_click(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        if !self.copy_selection(cx) {
            self.paste(cx);
        }
    }

    fn on_scroll(&mut self, event: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = self.cell.height;
        self.scroll_remainder += event.delta.pixel_delta(line_height).y;
        let lines = (self.scroll_remainder / line_height).trunc();
        if lines == 0. {
            return;
        }
        self.scroll_remainder -= line_height * lines;
        let lines = lines as i32;

        let mode = self.mode();
        if mode.intersects(TermMode::MOUSE_MODE) && mode.contains(TermMode::SGR_MOUSE) {
            // The program asked for the wheel itself.
            let (point, _) = self.grid_point(event.position);
            let button = if lines > 0 { 64 } else { 65 };
            let report = format!(
                "\x1b[<{button};{};{}M",
                point.column.0 + 1,
                point.line.0.max(0) + 1
            );
            self.send(report.repeat(lines.unsigned_abs() as usize).into_bytes());
        } else if mode.contains(TermMode::ALT_SCREEN) {
            // Full-screen programs have no scrollback; the wheel moves their cursor.
            let app = mode.contains(TermMode::APP_CURSOR);
            let arrow = match (lines > 0, app) {
                (true, true) => "\x1bOA",
                (true, false) => "\x1b[A",
                (false, true) => "\x1bOB",
                (false, false) => "\x1b[B",
            };
            self.send(arrow.repeat(lines.unsigned_abs() as usize).into_bytes());
        } else {
            self.term.lock().scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        if let Some(pty) = &self.pty {
            let _ = pty.send(Msg::Shutdown);
        }
    }
}

/// The bytes a terminal sends for a key, or `None` when the key is plain text.
fn key_bytes(keystroke: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let modifiers = keystroke.modifiers;
    let key = keystroke.key.as_str();
    // xterm's modifier parameter: 1 + shift(1) + alt(2) + ctrl(4).
    let modifier = 1
        + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control);
    let modified = modifier > 1;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);

    // Cursor keys: `CSI <letter>`, `SS3 <letter>` in application mode.
    let cursor = |letter: char| {
        if modified {
            format!("\x1b[1;{modifier}{letter}")
        } else if app_cursor {
            format!("\x1bO{letter}")
        } else {
            format!("\x1b[{letter}")
        }
    };
    // Editing and function keys: `CSI <number> ~`.
    let tilde = |number: u8| {
        if modified {
            format!("\x1b[{number};{modifier}~")
        } else {
            format!("\x1b[{number}~")
        }
    };
    let function = |letter: char| {
        if modified {
            format!("\x1b[1;{modifier}{letter}")
        } else {
            format!("\x1bO{letter}")
        }
    };

    let sequence = match key {
        // Shift+Enter and Alt+Enter mean "newline without submitting" to
        // programs that take multi-line input, such as Claude Code.
        "enter" if modifiers.shift || modifiers.alt => "\x1b\r".to_string(),
        "enter" => "\r".to_string(),
        "tab" if modifiers.shift => "\x1b[Z".to_string(),
        "tab" => "\t".to_string(),
        "escape" => "\x1b".to_string(),
        "backspace" if modifiers.control => "\x08".to_string(),
        "backspace" if modifiers.alt => "\x1b\x7f".to_string(),
        "backspace" => "\x7f".to_string(),
        "space" if modifiers.control => "\0".to_string(),
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => function('P'),
        "f2" => function('Q'),
        "f3" => function('R'),
        "f4" => function('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ => {
            let mut chars = key.chars();
            let (Some(ch), None) = (chars.next(), chars.next()) else {
                return None;
            };
            if modifiers.control {
                // Ctrl turns a letter into its control character.
                let control = match ch.to_ascii_lowercase() {
                    letter @ 'a'..='z' => letter as u8 - b'a' + 1,
                    '@' | '2' => 0,
                    '[' => 27,
                    '\\' => 28,
                    ']' => 29,
                    '^' | '6' => 30,
                    '_' | '-' => 31,
                    _ => return None,
                };
                let mut bytes = Vec::new();
                if modifiers.alt {
                    bytes.push(0x1b);
                }
                bytes.push(control);
                return Some(bytes);
            }
            if modifiers.alt {
                // Alt is sent as an escape prefix.
                let text = keystroke.key_char.clone().unwrap_or_else(|| ch.to_string());
                return Some(format!("\x1b{text}").into_bytes());
            }
            return None;
        }
    };
    Some(sequence.into_bytes())
}

// ---------------------------------------------------------------- colors

struct Palette {
    ansi: [u32; 16],
    foreground: Hsla,
    background: Hsla,
    cursor: Hsla,
    selection: Hsla,
}

impl Palette {
    fn new(theme: &Theme) -> Self {
        let ansi = if theme.is_dark() {
            [
                0x3f4451, 0xe05561, 0x8cc265, 0xd18f52, 0x4aa5f0, 0xc162de, 0x42b3c2, 0xd7dae0,
                0x5c6370, 0xff616e, 0xa5e075, 0xf0a45d, 0x4dc4ff, 0xde73ff, 0x4cd1e0, 0xf5f5f5,
            ]
        } else {
            [
                0x24292f, 0xcf222e, 0x116329, 0x9a6700, 0x0969da, 0x8250df, 0x1b7c83, 0x6e7781,
                0x57606a, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa, 0x8c959f,
            ]
        };
        Self {
            ansi,
            foreground: theme.foreground,
            background: theme.background,
            cursor: theme.foreground,
            selection: theme.selection,
        }
    }

    /// One of the 256 indexed colors, or a named color past them.
    fn indexed(&self, index: usize) -> Hsla {
        match index {
            0..=15 => rgb(self.ansi[index]).into(),
            // A 6×6×6 color cube.
            16..=231 => {
                let index = index - 16;
                let level = |value: usize| if value == 0 { 0 } else { 55 + 40 * value as u32 };
                let (r, g, b) = (level(index / 36), level(index / 6 % 6), level(index % 6));
                rgb(r << 16 | g << 8 | b).into()
            }
            // A 24-step gray ramp.
            232..=255 => {
                let gray = 8 + 10 * (index as u32 - 232);
                rgb(gray << 16 | gray << 8 | gray).into()
            }
            _ => match index - 256 {
                1 => self.background,
                2 => self.cursor,
                // Dim variants of the eight base colors.
                dim @ 3..=10 => {
                    let color: Hsla = rgb(self.ansi[dim - 3]).into();
                    color.opacity(0.7)
                }
                12 => self.foreground.opacity(0.7),
                _ => self.foreground,
            },
        }
    }

    fn rgb(&self, index: usize) -> Rgb {
        let color = Rgba::from(self.indexed(index));
        Rgb {
            r: (color.r * 255.) as u8,
            g: (color.g * 255.) as u8,
            b: (color.b * 255.) as u8,
        }
    }

    fn resolve(&self, color: Color, overrides: &alacritty_terminal::term::color::Colors) -> Hsla {
        let from_rgb = |color: Rgb| -> Hsla {
            rgb((color.r as u32) << 16 | (color.g as u32) << 8 | color.b as u32).into()
        };
        match color {
            Color::Spec(color) => from_rgb(color),
            Color::Named(name) => {
                let index = name as usize;
                overrides[index].map_or_else(|| self.indexed(index), from_rgb)
            }
            Color::Indexed(index) => {
                let index = index as usize;
                overrides[index].map_or_else(|| self.indexed(index), from_rgb)
            }
        }
    }
}

// ---------------------------------------------------------------- drawing

/// A stretch of cells drawn with one style.
struct Run {
    row: usize,
    col: usize,
    text: String,
    style: RunStyle,
}

#[derive(Clone, Copy, PartialEq)]
struct RunStyle {
    color: Hsla,
    bold: bool,
    italic: bool,
    underline: bool,
    strikethrough: bool,
}

/// What one frame draws, laid out against the bounds it was given.
struct Frame {
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    /// Bars drawn over the text: a beam, underline, or hollow cursor.
    overlays: Vec<(Bounds<Pixels>, Hsla)>,
}

fn layout(
    view: &Entity<TerminalView>,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) -> Frame {
    let theme = cx.theme();
    let palette = Palette::new(theme);
    let family = theme.mono_font_family.clone();
    let font_size = theme.mono_font_size;
    let base_font = font(family);
    let font_id = window.text_system().resolve_font(&base_font);
    let cell_width = window
        .text_system()
        .advance(font_id, font_size, 'm')
        .map(|advance| advance.width)
        .unwrap_or(font_size * 0.6);
    let line_height = px((f32::from(font_size) * 1.4).ceil());
    let cell = size(cell_width, line_height);

    let cols = ((bounds.size.width / cell_width) as usize).max(2);
    let lines = ((bounds.size.height / line_height) as usize).max(1);
    let focused = view.read(cx).focus.is_focused(window);
    view.update(cx, |view, _| view.set_size(cols, lines, cell, bounds.origin));
    let marked = view.read(cx).marked.clone();
    let term = view.read(cx).term.clone();
    let term = term.lock();
    let content = term.renderable_content();
    let offset = content.display_offset as i32;

    let cell_bounds = |row: usize, col: usize, width: usize| {
        Bounds::new(
            bounds.origin + point(cell_width * col as f32, line_height * row as f32),
            size(cell_width * width as f32, line_height),
        )
    };

    let cursor_row = content.cursor.point.line.0 + offset;
    let cursor_col = content.cursor.point.column.0;
    let cursor_shape = if content.mode.contains(TermMode::SHOW_CURSOR) {
        content.cursor.shape
    } else {
        CursorShape::Hidden
    };
    let block_cursor = focused && cursor_shape == CursorShape::Block;

    let mut backgrounds: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
    let mut runs: Vec<Run> = Vec::new();
    // The background being extended along the current row: row, start, end, color.
    let mut span: Option<(usize, usize, usize, Hsla)> = None;

    for indexed in content.display_iter {
        let row = indexed.point.line.0 + offset;
        if row < 0 || row as usize >= lines {
            continue;
        }
        let row = row as usize;
        let col = indexed.point.column.0;
        let cell = indexed.cell;
        if cell
            .flags
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

        let mut foreground = palette.resolve(cell.fg, content.colors);
        let mut background = palette.resolve(cell.bg, content.colors);
        let mut filled = !matches!(cell.bg, Color::Named(NamedColor::Background));
        if cell.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut foreground, &mut background);
            filled = true;
        }
        if cell.flags.contains(Flags::DIM) {
            foreground = foreground.opacity(0.7);
        }
        if content
            .selection
            .is_some_and(|selection| selection.contains(indexed.point))
        {
            background = palette.selection;
            filled = true;
        }
        if block_cursor && row as i32 == cursor_row && col == cursor_col {
            foreground = palette.background;
            background = palette.cursor;
            filled = true;
        }

        if filled {
            match &mut span {
                Some((span_row, _, end, color))
                    if *span_row == row && *end == col && *color == background =>
                {
                    *end = col + width;
                }
                _ => {
                    if let Some((row, start, end, color)) = span.take() {
                        backgrounds.push((cell_bounds(row, start, end - start), color));
                    }
                    span = Some((row, col, col + width, background));
                }
            }
        }

        let ch = cell.c;
        if ch == '\0' || cell.flags.contains(Flags::HIDDEN) {
            continue;
        }
        let style = RunStyle {
            color: foreground,
            bold: cell.flags.contains(Flags::BOLD),
            italic: cell.flags.contains(Flags::ITALIC),
            underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
            strikethrough: cell.flags.contains(Flags::STRIKEOUT),
        };
        // Only ASCII is sure to be exactly one cell wide in the font, so only
        // ASCII is drawn in stretches; anything else is placed on its own cell.
        let extends = ch.is_ascii()
            && runs.last().is_some_and(|run| {
                run.row == row
                    && run.style == style
                    && run.text.is_ascii()
                    && run.col + run.text.len() == col
            });
        if extends {
            runs.last_mut().unwrap().text.push(ch);
        } else if ch != ' ' || style.underline || style.strikethrough {
            let mut text = String::from(ch);
            text.extend(cell.zerowidth().into_iter().flatten());
            runs.push(Run {
                row,
                col,
                text,
                style,
            });
        }
    }
    if let Some((row, start, end, color)) = span.take() {
        backgrounds.push((cell_bounds(row, start, end - start), color));
    }
    drop(term);

    let mut overlays = Vec::new();
    let cursor_visible = cursor_row >= 0 && (cursor_row as usize) < lines;
    if cursor_visible {
        let row = cursor_row as usize;
        let cursor = cell_bounds(row, cursor_col, 1);
        let thickness = px(2.);
        let bar = |origin: Point<Pixels>, size: Size<Pixels>| (Bounds::new(origin, size), palette.cursor);
        match cursor_shape {
            CursorShape::Hidden => {}
            CursorShape::Beam => overlays.push(bar(cursor.origin, size(thickness, line_height))),
            CursorShape::Underline => overlays.push(bar(
                cursor.origin + point(px(0.), line_height - thickness),
                size(cell_width, thickness),
            )),
            // Unfocused, the block is drawn as an outline.
            _ if !block_cursor => {
                let thin = px(1.);
                overlays.push(bar(cursor.origin, size(cell_width, thin)));
                overlays.push(bar(
                    cursor.origin + point(px(0.), line_height - thin),
                    size(cell_width, thin),
                ));
                overlays.push(bar(cursor.origin, size(thin, line_height)));
                overlays.push(bar(
                    cursor.origin + point(cell_width - thin, px(0.)),
                    size(thin, line_height),
                ));
            }
            _ => {}
        }
        // Text still being composed sits at the cursor, underlined.
        if let Some(marked) = marked.filter(|marked| !marked.is_empty()) {
            let width = marked.chars().map(|ch| if ch.is_ascii() { 1 } else { 2 }).sum();
            backgrounds.push((cell_bounds(row, cursor_col, width), palette.background));
            runs.push(Run {
                row,
                col: cursor_col,
                text: marked,
                style: RunStyle {
                    color: palette.foreground,
                    bold: false,
                    italic: false,
                    underline: true,
                    strikethrough: false,
                },
            });
        }
    }

    let lines = runs
        .into_iter()
        .map(|run| {
            let mut font = base_font.clone();
            if run.style.bold {
                font.weight = FontWeight::BOLD;
            }
            if run.style.italic {
                font.style = FontStyle::Italic;
            }
            let text_run = TextRun {
                len: run.text.len(),
                font,
                color: run.style.color,
                background_color: None,
                underline: run.style.underline.then(|| UnderlineStyle {
                    thickness: px(1.),
                    color: Some(run.style.color),
                    wavy: false,
                }),
                strikethrough: run.style.strikethrough.then(|| StrikethroughStyle {
                    thickness: px(1.),
                    color: Some(run.style.color),
                }),
            };
            let origin = cell_bounds(run.row, run.col, 1).origin;
            let line = window
                .text_system()
                .shape_line(run.text.into(), font_size, &[text_run], None);
            (origin, line)
        })
        .collect();

    Frame {
        bounds,
        line_height,
        backgrounds,
        lines,
        overlays,
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        if let Some(error) = self.error.clone() {
            return div()
                .size_full()
                .p_3()
                .text_sm()
                .text_color(theme.danger)
                .child(error)
                .into_any_element();
        }
        let view = cx.entity();
        let focus = self.focus.clone();
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .size_full()
            .pl_3()
            .pt_1()
            .bg(theme.background)
            .cursor(CursorStyle::IBeam)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_right_click))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_scroll(cx.listener(Self::on_scroll))
            .child(
                canvas(
                    {
                        let view = view.clone();
                        move |bounds, window, cx| layout(&view, bounds, window, cx)
                    },
                    move |_, frame: Frame, window, cx| {
                        for (bounds, color) in frame.backgrounds {
                            window.paint_quad(fill(bounds, color));
                        }
                        for (origin, line) in &frame.lines {
                            let _ = line.paint(
                                *origin,
                                frame.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        for (bounds, color) in frame.overlays {
                            window.paint_quad(fill(bounds, color));
                        }
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(frame.bounds, view),
                            cx,
                        );
                    },
                )
                .size_full(),
            )
            .into_any_element()
    }
}

/// Typed and composed text. The terminal has no document of its own, so the
/// only text it reports is what an input method is composing.
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked
            .as_ref()
            .map(|marked| 0..marked.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        self.write(text.as_bytes().to_vec(), cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    /// Where the input method should put its candidate window: at the cursor.
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let term = self.term.lock();
        let cursor = term.grid().cursor.point;
        let row = (cursor.line.0 + term.grid().display_offset() as i32).max(0) as f32;
        Some(Bounds::new(
            self.origin + point(self.cell.width * cursor.column.0 as f32, self.cell.height * row),
            self.cell,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Keystroke, Palette, TermMode, key_bytes};

    fn bytes(key: &'static str, mode: TermMode) -> Option<String> {
        let keystroke = Keystroke::parse(key).unwrap();
        key_bytes(&keystroke, mode).map(|bytes| String::from_utf8(bytes).unwrap())
    }

    #[test]
    fn plain_letters_are_left_to_text_input() {
        assert_eq!(bytes("a", TermMode::NONE), None);
        assert_eq!(bytes("shift-a", TermMode::NONE), None);
    }

    #[test]
    fn control_keys_send_control_characters() {
        assert_eq!(bytes("ctrl-c", TermMode::NONE).as_deref(), Some("\x03"));
        assert_eq!(bytes("ctrl-d", TermMode::NONE).as_deref(), Some("\x04"));
        assert_eq!(bytes("ctrl-[", TermMode::NONE).as_deref(), Some("\x1b"));
        assert_eq!(bytes("alt-b", TermMode::NONE).as_deref(), Some("\x1bb"));
    }

    #[test]
    fn editing_keys_send_their_sequences() {
        assert_eq!(bytes("enter", TermMode::NONE).as_deref(), Some("\r"));
        assert_eq!(bytes("shift-enter", TermMode::NONE).as_deref(), Some("\x1b\r"));
        assert_eq!(bytes("backspace", TermMode::NONE).as_deref(), Some("\x7f"));
        assert_eq!(bytes("shift-tab", TermMode::NONE).as_deref(), Some("\x1b[Z"));
        assert_eq!(bytes("delete", TermMode::NONE).as_deref(), Some("\x1b[3~"));
        assert_eq!(bytes("f5", TermMode::NONE).as_deref(), Some("\x1b[15~"));
    }

    #[test]
    fn arrows_follow_the_cursor_key_mode_and_modifiers() {
        assert_eq!(bytes("up", TermMode::NONE).as_deref(), Some("\x1b[A"));
        assert_eq!(bytes("up", TermMode::APP_CURSOR).as_deref(), Some("\x1bOA"));
        assert_eq!(bytes("ctrl-right", TermMode::NONE).as_deref(), Some("\x1b[1;5C"));
        assert_eq!(bytes("shift-home", TermMode::APP_CURSOR).as_deref(), Some("\x1b[1;2H"));
    }

    #[test]
    fn indexed_colors_cover_the_cube_and_the_gray_ramp() {
        let palette = Palette {
            ansi: [0; 16],
            foreground: gpui_kit::white(),
            background: gpui_kit::black(),
            cursor: gpui_kit::white(),
            selection: gpui_kit::blue(),
        };
        let rgb_of = |index: usize| {
            let color = palette.rgb(index);
            (color.r, color.g, color.b)
        };
        assert_eq!(rgb_of(16), (0, 0, 0));
        assert_eq!(rgb_of(231), (255, 255, 255));
        assert_eq!(rgb_of(196), (255, 0, 0));
        assert_eq!(rgb_of(232), (8, 8, 8));
        assert_eq!(rgb_of(255), (238, 238, 238));
    }
}

/// Dumps what a shell sends at startup, escaped, to see every control sequence.
/// Run by hand: `cargo test raw_shell_output -- --ignored --nocapture`.
#[cfg(test)]
mod probe {
    use std::sync::Arc;

    use alacritty_terminal::event::{VoidListener, WindowSize};
    use alacritty_terminal::event_loop::EventLoop;
    use alacritty_terminal::sync::FairMutex;
    use alacritty_terminal::term::{Config, Term};
    use alacritty_terminal::tty;

    #[test]
    #[ignore]
    fn raw_shell_output() {
        let dir = std::env::temp_dir().join(format!("gust-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let (program, args) = super::default_shell();
        let options = tty::Options {
            shell: Some(tty::Shell::new(program, args)),
            working_directory: Some(dir.clone()),
            drain_on_exit: false,
            env: Default::default(),
            #[cfg(windows)]
            escape_args: true,
        };
        let size = WindowSize { num_lines: 24, num_cols: 120, cell_width: 8, cell_height: 16 };
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &super::GridSize { cols: 120, lines: 24 },
            VoidListener,
        )));
        let pty = tty::new(&options, size, 0).unwrap();
        let event_loop = EventLoop::new(term, VoidListener, pty, false, true).unwrap();
        let _sender = event_loop.channel();
        event_loop.spawn();
        std::thread::sleep(std::time::Duration::from_secs(5));
        let raw = std::fs::read(dir.join("alacritty.recording")).unwrap_or_default();
        println!("RAW<<{}>>", String::from_utf8_lossy(&raw).escape_debug());
    }

    /// Runs a program the way the run console does, then stops it.
    /// Run by hand: `GUST_PROGRAM=<exe> cargo test console_program -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn console_program() {
        use alacritty_terminal::event_loop::Msg;
        let program = std::env::var("GUST_PROGRAM").unwrap();
        let dir = std::env::temp_dir();
        let options = tty::Options {
            shell: Some(tty::Shell::new(program.clone(), vec!["-x".into(), "two words".into()])),
            working_directory: Some(dir),
            drain_on_exit: true,
            env: [("APP_ENV".to_string(), "probe".to_string())].into(),
            #[cfg(windows)]
            escape_args: true,
        };
        let size = WindowSize { num_lines: 24, num_cols: 120, cell_width: 8, cell_height: 16 };
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &super::GridSize { cols: 120, lines: 24 },
            VoidListener,
        )));
        let pty = tty::new(&options, size, 0).unwrap();
        let event_loop = EventLoop::new(term.clone(), VoidListener, pty, false, false).unwrap();
        let sender = event_loop.channel();
        event_loop.spawn();
        std::thread::sleep(std::time::Duration::from_secs(2));
        {
            let term = term.lock();
            let grid = alacritty_terminal::grid::Dimensions::screen_lines(&*term);
            let text = term.bounds_to_string(
                alacritty_terminal::index::Point::new(alacritty_terminal::index::Line(0), alacritty_terminal::index::Column(0)),
                alacritty_terminal::index::Point::new(alacritty_terminal::index::Line(grid as i32 - 1), alacritty_terminal::index::Column(119)),
            );
            println!("SCREEN<<{}>>", text.trim_end());
        }
        let name = std::path::Path::new(&program).file_stem().unwrap().to_string_lossy().into_owned();
        let alive = || {
            let out = std::process::Command::new("tasklist").output().unwrap();
            String::from_utf8_lossy(&out.stdout).contains(&name)
        };
        println!("ALIVE before stop: {}", alive());
        let _ = sender.send(Msg::Shutdown);
        std::thread::sleep(std::time::Duration::from_secs(2));
        println!("ALIVE after stop: {}", alive());
    }
}
