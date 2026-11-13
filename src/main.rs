#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod compare;
mod complete;
mod file_tree;
mod finder;
mod format;
mod git;
mod goindex;
mod history;
mod lang;
mod proc;
mod recent;
mod run;
mod settings;
mod terminal;
mod theme;
mod ui;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::{ThemeMode, TitleBar};
use gpui_kit::*;

use workspace::Workspace;

fn main() {
    // `gust <folder> [file…]` opens that folder and those files, `gust <file>`
    // opens the file in its own folder; with no argument, the current folder.
    let mut args: Vec<PathBuf> = std::env::args()
        .skip(1)
        .map(|arg| absolute(PathBuf::from(arg)))
        .collect();
    let root = match args.first() {
        Some(first) if first.is_dir() => args.remove(0),
        Some(first) => first.parent().map(PathBuf::from).unwrap_or_default(),
        None => absolute(PathBuf::from(".")),
    };
    let files = args;

    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            theme::init(ThemeMode::Dark, cx);
            workspace::init(cx);
            terminal::init(cx);

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1280.), px(820.)),
                    cx,
                ))),
                window_min_size: Some(size(px(640.), px(400.))),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(root, window, cx);
                    for file in files {
                        workspace.open(file, window, cx);
                    }
                    workspace
                })
            })
            .expect("failed to open window");
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
