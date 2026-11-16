#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod compare;
mod expand;
mod complete;
mod file_tree;
mod finder;
mod format;
mod generate;
mod git;
mod goindex;
mod history;
mod lang;
mod lines;
mod local_history;
mod merge;
mod problems;
mod proc;
mod recent;
mod run;
mod settings;
mod terminal;
mod theme;
mod ui;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::ThemeMode;


fn main() {
    // `gust <folder> [file…]` opens that folder and those files, `gust <file>`
    // opens the file in its own folder. With no argument, the current folder
    // when started from a shell in it, or the last project when started by
    // double-clicking or from a menu.
    let mut args: Vec<PathBuf> = std::env::args()
        .skip(1)
        .map(|arg| absolute(PathBuf::from(arg)))
        .collect();
    let root = match args.first() {
        Some(first) if first.is_dir() => args.remove(0),
        Some(first) => first.parent().map(PathBuf::from).unwrap_or_default(),
        None => {
            let here = absolute(PathBuf::from("."));
            if started_from_a_shell(&here) {
                here
            } else {
                recent::last().unwrap_or(here)
            }
        }
    };
    let files = args;

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let app = settings::load_app();
            theme::set_font_size(app.font_size, cx);
            theme::init(if app.dark { ThemeMode::Dark } else { ThemeMode::Light }, cx);
            complete::set_hover_docs(app.hover_docs);
            workspace::init(cx);
            terminal::init(cx);
            workspace::apply_keymap(&app.keymap, cx);

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            workspace::open_window(root, files, cx);
            cx.activate(true);
        });
}

/// The path made absolute. `canonicalize` on Windows returns `\\?\C:\…`, which
/// tools such as git and gofmt do not accept as a working directory, so that
/// prefix is dropped.
fn absolute(path: PathBuf) -> PathBuf {
    let Ok(path) = std::fs::canonicalize(&path) else {
        return path;
    };
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(stripped) if !stripped.starts_with("UNC") => PathBuf::from(stripped),
        _ => path,
    }
}

/// Whether `here`, the current folder, was chosen by whoever started Gust:
/// a double-click starts it in its own folder, a menu in the system's.
fn started_from_a_shell(here: &std::path::Path) -> bool {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from));
    let system = std::env::var_os("SystemRoot").map(PathBuf::from);
    let same = |dir: &Option<PathBuf>| {
        dir.as_ref().is_some_and(|dir| {
            here.to_string_lossy().to_lowercase() == dir.to_string_lossy().to_lowercase()
                || here.starts_with(dir)
        })
    };
    !same(&exe_dir) && !same(&system)
}
