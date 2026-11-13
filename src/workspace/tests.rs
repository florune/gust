//! Workflows through the real workspace in a headless window: keys go through
//! the bound actions and the editor's own input handling.

use std::path::PathBuf;

use gpui_kit::base::Root;
use gpui_kit::component::input::{EditorState, Position};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, px, size,
};

use super::Workspace;

const GO_MOD: (&str, &str) = ("go.mod", "module example.com/demo\n\ngo 1.22\n");

struct Fixture {
    root: PathBuf,
    window: WindowHandle<Root>,
    workspace: Entity<Workspace>,
}

impl Fixture {
    /// A project holding `files`, with `open` shown in the editor.
    fn new(cx: &mut TestAppContext, name: &str, files: &[(&str, &str)], open: &str) -> Self {
        let root = std::env::temp_dir().join(format!("gust-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, content) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        Self::open(cx, root, open)
    }

    fn open(cx: &mut TestAppContext, root: PathBuf, open: &str) -> Self {
        Self::open_sized(cx, root, open, px(1000.))
    }

    fn open_sized(cx: &mut TestAppContext, root: PathBuf, open: &str, width: gpui_kit::Pixels) -> Self {
        cx.update(|cx| {
            gpui_kit::init(cx);
            // The other way round from `main`: the terminal must keep its
            // keys whichever is registered first.
            crate::terminal::init(cx);
            super::init(cx);
        });
        let (window, workspace) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(width, px(700.)),
                })),
                ..Default::default()
            };
            let file = root.join(open);
            let root = root.clone();
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(root, window, cx);
                    // Tests type into Markdown files; the preview is tested on its own.
                    workspace.markdown_view = super::DocView::Edit;
                    workspace.open(file, window, cx);
                    workspace
                })
            })
            .expect("open test window")
        });
        let window = window.downcast::<Root>().expect("Base Root");
        cx.update_window(window.into(), |_, window, _| window.activate_window())
            .unwrap();
        let fixture = Self {
            root,
            window,
            workspace,
        };
        fixture.settle(cx);
        fixture
    }

    /// A project like [`Fixture::new`], committed to a fresh repository first.
    fn in_repository(
        cx: &mut TestAppContext,
        name: &str,
        files: &[(&str, &str)],
        open: &str,
    ) -> Self {
        let root = std::env::temp_dir().join(format!("gust-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for (path, content) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "Test"]);
        git(&root, &["config", "core.autocrlf", "false"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "--quiet", "-m", "first"]);
        Self::open(cx, root, open)
    }

    /// The paths and letters in the changes panel.
    fn changes(&self, cx: &mut TestAppContext) -> Vec<(String, &'static str)> {
        self.workspace.read_with(cx, |workspace, _| {
            workspace
                .git
                .iter()
                .flat_map(|status| &status.changes)
                .map(|change| (change.path.clone(), change.kind.letter()))
                .collect()
        })
    }

    /// Let background work (formatter, index) finish and draw the result.
    fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.window.into(), |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn editor(&self, cx: &mut TestAppContext) -> Entity<EditorState> {
        self.workspace.read_with(cx, |workspace, _| {
            let active = workspace.active.expect("a file is open");
            workspace.docs[active].editor.clone()
        })
    }

    fn active_file(&self, cx: &mut TestAppContext) -> String {
        self.workspace.read_with(cx, |workspace, _| {
            let active = workspace.active.expect("a file is open");
            workspace.docs[active]
                .path
                .strip_prefix(&self.root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
    }

    fn text(&self, cx: &mut TestAppContext) -> String {
        self.editor(cx).read_with(cx, |editor, _| editor.text().to_string())
    }

    fn cursor(&self, cx: &mut TestAppContext) -> (u32, u32) {
        self.editor(cx).read_with(cx, |editor, _| {
            let position = editor.cursor_position();
            (position.line, position.character)
        })
    }

    fn set_cursor(&self, line: u32, character: u32, cx: &mut TestAppContext) {
        let editor = self.editor(cx);
        cx.update_window(self.window.into(), |_, window, cx| {
            editor.update(cx, |editor, cx| {
                editor.set_cursor_position(Position::new(line, character), window, cx)
            });
        })
        .unwrap();
        self.settle(cx);
    }

    fn input(&self, text: &str, cx: &mut TestAppContext) {
        cx.update_window(self.window.into(), |_, window, cx| window.input(text, cx))
            .unwrap();
        self.settle(cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        cx.update_window(self.window.into(), |_, window, cx| window.press(key, cx))
            .unwrap();
        self.settle(cx);
    }

    /// The labels in the open completion menu, empty when it is closed.
    fn menu(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.editor(cx).read_with(cx, |editor, _| {
            let menu = editor.completion_menu_state();
            if !menu.open {
                return Vec::new();
            }
            menu.items.iter().map(|item| item.label.clone()).collect()
        })
    }

    fn open_file(&self, file: &str, cx: &mut TestAppContext) {
        let path = self.root.join(file);
        cx.update_window(self.window.into(), |_, window, cx| {
            self.workspace
                .update(cx, |workspace, cx| workspace.open(path, window, cx));
        })
        .unwrap();
        self.settle(cx);
    }

    /// The open files in tab order.
    fn tabs(&self, cx: &mut TestAppContext) -> Vec<String> {
        self.workspace.read_with(cx, |workspace, _| {
            workspace
                .docs
                .iter()
                .map(|doc| doc.title.to_string())
                .collect()
        })
    }

    fn on_disk(&self, file: &str) -> String {
        std::fs::read_to_string(self.root.join(file)).unwrap()
    }

    fn dirty(&self, cx: &mut TestAppContext) -> bool {
        self.workspace.read_with(cx, |workspace, _| {
            workspace.docs[workspace.active.expect("a file is open")].dirty
        })
    }
}

/// Run git in `root`, as the fixture's setup or to look at its history.
fn git(root: &std::path::Path, args: &[&str]) -> String {
    let output = crate::proc::command("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[gpui_kit::test]
fn typing_a_letter_offers_names_and_enter_inserts_one(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "complete",
        &[
            GO_MOD,
            (
                "main.go",
                "package main\n\nconst version = \"0.1.0\"\n\nfunc verify() bool { return true }\n\nfunc main() {\n\tprintln()\n}\n",
            ),
        ],
        "main.go",
    );
    fixture.set_cursor(7, 9, cx);

    fixture.input("v", cx);
    assert_eq!(fixture.menu(cx), ["version", "verify", "var"]);

    fixture.input("e", cx);
    fixture.input("r", cx);
    assert_eq!(fixture.menu(cx), ["version", "verify"]);

    fixture.press("down", cx);
    fixture.press("enter", cx);
    assert!(fixture.menu(cx).is_empty());
    assert!(
        fixture.text(cx).contains("\tprintln(verify)\n"),
        "{}",
        fixture.text(cx)
    );
    assert!(fixture.dirty(cx));
}

#[gpui_kit::test]
fn a_dot_offers_the_members_of_what_precedes_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "members",
        &[
            GO_MOD,
            (
                "main.go",
                "package main\n\ntype server struct {\n\taddr string\n\tport int\n}\n\nfunc (s *server) start() error { return nil }\n\nfunc main() {\n\tsrv := &server{}\n\tsrv\n}\n",
            ),
        ],
        "main.go",
    );
    fixture.set_cursor(11, 4, cx);

    fixture.input(".", cx);
    assert_eq!(fixture.menu(cx), ["addr", "port", "start"]);

    fixture.input("s", cx);
    assert_eq!(fixture.menu(cx), ["start"]);
    fixture.press("enter", cx);
    assert!(
        fixture.text(cx).contains("\tsrv.start\n"),
        "{}",
        fixture.text(cx)
    );
}

#[gpui_kit::test]
fn saving_a_go_file_formats_it(cx: &mut TestAppContext) {
    let unformatted = "package main\n\nfunc  main( ) {\nx:=1\n_ = x\n}\n";
    let fixture = Fixture::new(cx, "format", &[GO_MOD, ("main.go", unformatted)], "main.go");
    fixture.set_cursor(3, 4, cx);

    fixture.press("ctrl-s", cx);

    let formatted = "package main\n\nfunc main() {\n\tx := 1\n\t_ = x\n}\n";
    assert_eq!(fixture.text(cx), formatted);
    assert_eq!(fixture.on_disk("main.go"), formatted);
    assert!(!fixture.dirty(cx));
    // The caret stays at the end of its line instead of jumping elsewhere.
    assert_eq!(fixture.cursor(cx), (3, 7));
}

#[gpui_kit::test]
fn a_go_file_that_does_not_parse_is_still_saved(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "broken",
        &[GO_MOD, ("main.go", "package main\n\nfunc main() {\n}\n")],
        "main.go",
    );
    fixture.set_cursor(2, 13, cx);
    fixture.input("\n\tx :=", cx);

    fixture.press("ctrl-s", cx);

    assert!(fixture.on_disk("main.go").contains("x :="));
    assert!(!fixture.dirty(cx));
    let notice = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.notice.clone());
    assert!(notice.is_some(), "the formatter's complaint is shown");
}

#[gpui_kit::test]
fn formatting_json_reindents_it_without_saving(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "json",
        &[("config.json", "{\"name\":\"gust\",\"ports\":[1,2]}")],
        "config.json",
    );

    fixture.press("ctrl-alt-l", cx);

    assert_eq!(
        fixture.text(cx),
        "{\n  \"name\": \"gust\",\n  \"ports\": [\n    1,\n    2\n  ]\n}\n"
    );
    assert_eq!(
        fixture.on_disk("config.json"),
        "{\"name\":\"gust\",\"ports\":[1,2]}"
    );
    assert!(fixture.dirty(cx));
}

#[gpui_kit::test]
fn going_to_a_definition_opens_the_file_that_declares_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "definition",
        &[
            GO_MOD,
            ("util.go", "package main\n\n// helper helps.\nfunc helper() {}\n"),
            ("main.go", "package main\n\nfunc main() {\n\thelper()\n}\n"),
        ],
        "main.go",
    );
    fixture.set_cursor(3, 3, cx);

    fixture.press("ctrl-b", cx);

    assert_eq!(fixture.active_file(cx), "util.go");
    assert_eq!(fixture.cursor(cx), (3, 5));
}

#[gpui_kit::test]
fn the_terminal_keeps_its_keys_from_the_editor(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "terminal", &[("notes.md", "one\n")], "notes.md");
    let state = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| {
            (
                workspace.terminal_visible,
                workspace.terminals.len(),
                workspace.docs.len(),
            )
        })
    };

    fixture.press("ctrl-`", cx);
    assert_eq!(state(cx), (true, 1, 1));

    // Ctrl+W erases a word in the shell; it must not close the editor tab.
    fixture.press("ctrl-w", cx);
    assert_eq!(state(cx), (true, 1, 1));

    fixture.press("ctrl-`", cx);
    assert_eq!(state(cx), (false, 1, 1));
    // Back in the editor, the same key closes the tab again.
    fixture.press("ctrl-w", cx);
    assert_eq!(state(cx), (false, 1, 0));
}

#[gpui_kit::test]
fn saved_edits_show_up_as_changes_and_can_be_committed(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(
        cx,
        "commit",
        &[("notes.md", "one\ntwo\n"), ("other.md", "keep\n")],
        "notes.md",
    );
    assert_eq!(fixture.changes(cx), []);

    fixture.set_cursor(0, 3, cx);
    fixture.input("!", cx);
    fixture.press("ctrl-s", cx);
    std::fs::write(fixture.root.join("new.md"), "fresh\n").unwrap();
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.refresh_git(cx));
    fixture.settle(cx);
    assert_eq!(
        fixture.changes(cx),
        [("new.md".to_string(), "U"), ("notes.md".to_string(), "M")]
    );

    // Leave the new file out of the commit.
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.workspace.update(cx, |workspace, cx| {
            workspace.excluded.insert("new.md".to_string());
            workspace
                .commit_message
                .update(cx, |input, cx| input.set_value("second", window, cx));
            workspace.commit(window, cx);
        });
    })
    .unwrap();
    fixture.settle(cx);

    assert_eq!(fixture.changes(cx), [("new.md".to_string(), "U")]);
    assert_eq!(git(&fixture.root, &["log", "--format=%s"]), "second\nfirst\n");
    let message = fixture
        .workspace
        .read_with(cx, |workspace, cx| workspace.commit_message.read(cx).value());
    assert_eq!(message, "");
}

#[gpui_kit::test]
fn a_changed_file_opens_as_a_diff(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "diff", &[("notes.md", "one\ntwo\n")], "notes.md");
    std::fs::write(fixture.root.join("notes.md"), "one\n2\nthree\n").unwrap();
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.refresh_git(cx));
    fixture.settle(cx);

    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.workspace.update(cx, |workspace, cx| {
            workspace.open_diff(Some("notes.md".to_string()), window, cx)
        });
    })
    .unwrap();
    fixture.settle(cx);

    let (active, added, removed) = fixture.workspace.read_with(cx, |workspace, cx| {
        let _ = cx;
        let tab = workspace.diff.as_ref().expect("the diff tab is open");
        let (added, removed) = tab.counts();
        (workspace.diff_active, added, removed)
    });
    assert_eq!((active, added, removed), (true, 2, 1));

    // Ctrl+W closes the diff, not the file behind it.
    fixture.press("ctrl-w", cx);
    let (diff_open, files_open) = fixture
        .workspace
        .read_with(cx, |workspace, _| (workspace.diff.is_some(), workspace.docs.len()));
    assert_eq!((diff_open, files_open), (false, 1));
}

#[gpui_kit::test]
fn a_file_changed_on_disk_is_reloaded_unless_it_has_unsaved_edits(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "reload", &[("notes.md", "one\ntwo\n")], "notes.md");
    let changed = |cx: &mut TestAppContext| {
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture
                .workspace
                .update(cx, |workspace, cx| workspace.files_changed(window, cx));
        })
        .unwrap();
        fixture.settle(cx);
    };

    // Something else rewrites the file: the editor follows.
    std::fs::write(fixture.root.join("notes.md"), "one\ntwo\nthree\n").unwrap();
    changed(cx);
    assert_eq!(fixture.text(cx), "one\ntwo\nthree\n");
    assert!(!fixture.dirty(cx));

    // With unsaved edits, the editor keeps them and says so.
    fixture.set_cursor(0, 3, cx);
    fixture.input("!", cx);
    std::fs::write(fixture.root.join("notes.md"), "replaced\n").unwrap();
    changed(cx);
    assert_eq!(fixture.text(cx), "one!\ntwo\nthree\n");
    assert!(fixture.dirty(cx));
    let notice = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.notice.clone());
    assert!(notice.is_some());
}

#[gpui_kit::test]
fn files_and_text_are_found_from_the_keyboard(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "find",
        &[
            ("notes.md", "start here\n"),
            ("internal/server/server.go", "package server\n\nfunc Start() {}\n"),
            ("internal/store/store.go", "package store\n"),
        ],
        "notes.md",
    );
    let hits = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| {
            workspace
                .palette
                .iter()
                .flat_map(|palette| &palette.hits)
                .map(|hit| hit.path.clone())
                .collect::<Vec<_>>()
        })
    };

    fixture.press("ctrl-p", cx);
    fixture.input("sto", cx);
    assert_eq!(hits(cx), ["internal/store/store.go"]);
    fixture.press("enter", cx);
    assert_eq!(fixture.active_file(cx), "internal/store/store.go");
    assert!(hits(cx).is_empty(), "the box closes once a file is chosen");

    fixture.press("ctrl-shift-f", cx);
    fixture.input("func start", cx);
    // Text search waits for typing to pause.
    cx.executor().advance_clock(std::time::Duration::from_millis(200));
    fixture.settle(cx);
    assert_eq!(hits(cx), ["internal/server/server.go"]);
    fixture.press("enter", cx);
    assert_eq!(fixture.active_file(cx), "internal/server/server.go");
    assert_eq!(fixture.cursor(cx), (2, 0));

    // Escape puts the keyboard back in the editor.
    fixture.press("ctrl-p", cx);
    fixture.press("escape", cx);
    fixture.input("// ", cx);
    assert!(fixture.text(cx).contains("// func Start() {}"));
}

#[gpui_kit::test]
fn tabs_are_reordered_by_dragging(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "drag",
        &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n")],
        "a.md",
    );
    fixture.open_file("b.md", cx);
    fixture.open_file("c.md", cx);
    assert_eq!(fixture.tabs(cx), ["a.md", "b.md", "c.md"]);
    assert_eq!(fixture.active_file(cx), "c.md");

    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.drag_to(("tab-slot", 0usize), ("tab-slot", 2usize), cx)
    })
    .unwrap();
    fixture.settle(cx);
    assert_eq!(fixture.tabs(cx), ["b.md", "c.md", "a.md"]);
    // The file being edited stays the active one wherever it moved.
    assert_eq!(fixture.active_file(cx), "c.md");

    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.drag_to(("tab-slot", 2usize), ("tab-slot", 0usize), cx)
    })
    .unwrap();
    fixture.settle(cx);
    assert_eq!(fixture.tabs(cx), ["a.md", "b.md", "c.md"]);
}

#[gpui_kit::test]
fn the_diff_tab_is_dragged_like_any_other(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(
        cx,
        "difftab",
        &[("a.md", "a\n"), ("b.md", "b\n")],
        "a.md",
    );
    fixture.open_file("b.md", cx);
    std::fs::write(fixture.root.join("a.md"), "a!\n").unwrap();
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.refresh_git(cx));
    fixture.settle(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.workspace.update(cx, |workspace, cx| {
            workspace.open_diff(Some("a.md".to_string()), window, cx)
        });
    })
    .unwrap();
    fixture.settle(cx);
    let order = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| {
            workspace
                .tab_order()
                .into_iter()
                .map(|item| match item {
                    super::tabs::TabItem::Doc(ix) => workspace.docs[ix].title.to_string(),
                    super::tabs::TabItem::Diff => "diff".to_string(),
                })
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(order(cx), ["a.md", "b.md", "diff"]);

    // A file dropped onto the diff tab takes its place.
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.drag_to(("tab-slot", 0usize), ("tab-slot", 2usize), cx)
    })
    .unwrap();
    fixture.settle(cx);
    assert_eq!(order(cx), ["b.md", "diff", "a.md"]);

    // The diff tab moves too.
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.drag_to(("tab-slot", 1usize), ("tab-slot", 0usize), cx)
    })
    .unwrap();
    fixture.settle(cx);
    assert_eq!(order(cx), ["diff", "b.md", "a.md"]);

    // Closing a file before it keeps the diff tab in place.
    cx.update_window(fixture.window.into(), |_, window, cx| {
        let path = fixture.root.join("b.md");
        fixture
            .workspace
            .update(cx, |workspace, cx| workspace.close_path(&path, window, cx));
    })
    .unwrap();
    fixture.settle(cx);
    assert_eq!(order(cx), ["diff", "a.md"]);
}

#[gpui_kit::test]
fn the_tab_menu_closes_other_right_and_all_tabs(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "tabmenu",
        &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n"), ("d.md", "d\n")],
        "a.md",
    );
    for file in ["b.md", "c.md", "d.md"] {
        fixture.open_file(file, cx);
    }
    // An unsaved edit in a tab that is about to close.
    fixture.set_cursor(0, 1, cx);
    fixture.input("!", cx);

    let run = |cx: &mut TestAppContext, action: fn(&mut Workspace, &std::path::Path, &mut gpui_kit::Window, &mut gpui_kit::Context<Workspace>), file: &str| {
        let path = fixture.root.join(file);
        cx.update_window(fixture.window.into(), |_, window, cx| {
            fixture
                .workspace
                .update(cx, |workspace, cx| action(workspace, &path, window, cx));
        })
        .unwrap();
        fixture.settle(cx);
    };

    run(cx, Workspace::close_to_the_right, "b.md");
    assert_eq!(fixture.tabs(cx), ["a.md", "b.md", "c.md"].map(String::from).to_vec()[..2]);
    assert_eq!(fixture.on_disk("d.md"), "d!\n", "closing saves");

    fixture.open_file("c.md", cx);
    run(cx, Workspace::close_others, "b.md");
    assert_eq!(fixture.tabs(cx), ["b.md"]);
    assert_eq!(fixture.active_file(cx), "b.md");

    run(cx, |workspace, _, window, cx| workspace.close_all(window, cx), "b.md");
    assert!(fixture.tabs(cx).is_empty());
}

#[gpui_kit::test]
fn the_git_log_lists_branches_and_opens_a_commit_as_a_diff(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "log", &[("notes.md", "one\n")], "notes.md");
    let commit = |file: &str, content: &str, author: &str, message: &str| {
        std::fs::write(fixture.root.join(file), content).unwrap();
        git(&fixture.root, &["add", file]);
        git(
            &fixture.root,
            &["-c", &format!("user.name={author}"), "commit", "--quiet", "-m", message],
        );
    };
    git(&fixture.root, &["switch", "--quiet", "-c", "feature"]);
    commit("feature.md", "f\n", "Lina", "feature work");
    git(&fixture.root, &["switch", "--quiet", "main"]);
    commit("notes.md", "one\ntwo\n", "Wei", "second note");
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.refresh_git(cx));
    fixture.settle(cx);

    let subjects = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| {
            workspace.log.as_ref().map(|log| log.subjects()).unwrap_or_default()
        })
    };

    fixture.press("alt-9", cx);
    let mut all = subjects(cx);
    all.sort();
    assert_eq!(all, ["feature work", "first", "second note"]);
    let branches = fixture.workspace.read_with(cx, |workspace, _| {
        let log = workspace.log.as_ref().unwrap();
        log.branches.iter().map(|b| b.name.clone()).collect::<Vec<_>>()
    });
    assert_eq!(branches, ["feature", "main"]);

    // What is checked out is selected from the start, with its details.
    let (selected, files) = fixture.workspace.read_with(cx, |workspace, _| {
        let log = workspace.log.as_ref().unwrap();
        (
            log.selected.map(|ix| log.commits[ix].subject.clone()),
            log.details.as_ref().map(|d| d.files.len()),
        )
    });
    assert_eq!(selected.as_deref(), Some("second note"));
    assert_eq!(files, Some(1));

    // One branch at a time.
    fixture.workspace.update(cx, |workspace, cx| {
        workspace.set_log_scope(crate::history::Scope::Branch("refs/heads/feature".into()), cx)
    });
    fixture.settle(cx);
    assert_eq!(subjects(cx), ["feature work", "first"]);

    // A colleague's commits by name.
    fixture.workspace.update(cx, |workspace, cx| {
        workspace.set_log_scope(crate::history::Scope::All, cx)
    });
    fixture.settle(cx);
    let filter = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.log.as_ref().unwrap().filter.clone());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        filter.update(cx, |input, cx| input.focus(window, cx));
    })
    .unwrap();
    fixture.input("lina", cx);
    assert_eq!(subjects(cx), ["feature work"]);

    // Its file opens as that commit's diff.
    let ix = fixture.workspace.read_with(cx, |workspace, _| {
        let log = workspace.log.as_ref().unwrap();
        log.visible[0]
    });
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.select_commit(ix, cx));
    fixture.settle(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.workspace.update(cx, |workspace, cx| {
            let file = workspace.log.as_ref().unwrap().details.as_ref().unwrap().files[0].clone();
            workspace.open_commit_file(Some(file), window, cx)
        });
    })
    .unwrap();
    fixture.settle(cx);
    let (title, added) = fixture.workspace.read_with(cx, |workspace, cx| {
        let _ = cx;
        let tab = workspace.diff.as_ref().expect("the diff tab is open");
        (tab.title.to_string(), tab.counts().0)
    });
    assert!(title.ends_with("· feature.md"), "{title}");
    assert_eq!(added, 1);

    // Alt+9 again puts the panel away.
    fixture.press("alt-9", cx);
    let shown = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.log_visible());
    assert!(!shown);
}

#[gpui_kit::test]
fn hidden_paths_leave_the_tree_and_the_finder_and_stay_hidden(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "hidden",
        &[
            ("main.go", "package main\n"),
            ("docs/spec.md", "spec\n"),
            ("deploy.sh", "echo\n"),
        ],
        "main.go",
    );
    let tree = |cx: &mut TestAppContext| {
        fixture
            .workspace
            .read_with(cx, |workspace, cx| workspace.tree.read(cx).visible_paths())
    };
    assert_eq!(tree(cx), ["docs", "deploy.sh", "main.go"]);

    let hide = |cx: &mut TestAppContext, path: &str, hide: bool| {
        let path = fixture.root.join(path);
        fixture
            .workspace
            .update(cx, |workspace, cx| workspace.set_path_hidden(&path, hide, cx));
        fixture.settle(cx);
    };
    hide(cx, "docs", true);
    hide(cx, "deploy.sh", true);
    assert_eq!(tree(cx), ["main.go"]);

    // The file finder does not offer them either.
    fixture.press("ctrl-p", cx);
    fixture.input("spec", cx);
    let hits = fixture.workspace.read_with(cx, |workspace, _| {
        workspace.palette.as_ref().map(|p| p.hits.len()).unwrap_or(0)
    });
    assert_eq!(hits, 0);
    fixture.press("escape", cx);

    // Shown on request, and shown again for good.
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.tree.update(cx, |tree, cx| tree.toggle_show_hidden(cx)));
    assert_eq!(tree(cx), ["docs", "deploy.sh", "main.go"]);
    hide(cx, "deploy.sh", false);
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.tree.update(cx, |tree, cx| tree.toggle_show_hidden(cx)));
    assert_eq!(tree(cx), ["deploy.sh", "main.go"]);

    // The choice belongs to the project and outlives the window.
    assert_eq!(crate::settings::load(&fixture.root).hidden, ["docs"]);
}

#[gpui_kit::test]
fn markdown_opens_as_a_preview_and_switches_to_source(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "markdown",
        &[("README.md", "# Title\n\nSome *text*.\n"), ("main.go", "package main\n")],
        "main.go",
    );
    fixture.workspace.update(cx, |workspace, _| {
        workspace.markdown_view = super::DocView::Preview
    });
    fixture.open_file("README.md", cx);
    let view = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| {
            workspace.docs[workspace.active.unwrap()].view
        })
    };
    assert_eq!(view(cx), super::DocView::Preview);

    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.click("view-split", cx);
    })
    .unwrap();
    fixture.settle(cx);
    assert_eq!(view(cx), super::DocView::Split);

    // In the split, typing edits the source.
    fixture.set_cursor(0, 7, cx);
    fixture.input(" here", cx);
    assert!(fixture.text(cx).starts_with("# Title here"));

    // Other files are plain editors.
    fixture.open_file("main.go", cx);
    assert_eq!(view(cx), super::DocView::Edit);
}

#[gpui_kit::test]
fn a_wide_preview_keeps_the_same_margins_from_frame_to_frame(cx: &mut TestAppContext) {
    let root = std::env::temp_dir().join(format!("gust-ui-wide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("main.go"), "package main
").unwrap();
    let text: String = (0..40)
        .map(|n| format!("## Section {n}

A paragraph long enough to fill the reading width of the preview, and then some more words so that it wraps.

"))
        .collect();
    std::fs::write(root.join("notes.md"), text).unwrap();
    let fixture = Fixture::open_sized(cx, root, "main.go", px(1900.));
    fixture.workspace.update(cx, |workspace, _| {
        workspace.markdown_view = super::DocView::Preview
    });
    fixture.open_file("notes.md", cx);

    let mut widths = Vec::new();
    for _ in 0..8 {
        cx.executor().advance_clock(std::time::Duration::from_millis(17));
        fixture.settle(cx);
        widths.push(fixture.workspace.read_with(cx, |workspace, cx| {
            let doc = &workspace.docs[workspace.active.unwrap()];
            doc.preview.as_ref().unwrap().text_width(cx)
        }));
    }
    // Once measured, the text keeps one width; it used to alternate between
    // the reading width and the whole pane, every frame.
    let settled = &widths[2..];
    assert!(settled.iter().all(|w| *w == settled[0]), "{widths:?}");
    assert!(settled[0] > px(800.) && settled[0] < px(1100.), "{widths:?}");
}

#[gpui_kit::test]
fn the_preview_can_be_searched(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "mdsearch",
        &[
            ("main.go", "package main\n"),
            (
                "notes.md",
                "# Snapshot\n\nThe **snapshot** is sent first.\n\n- snapshot again\n- other\n",
            ),
        ],
        "main.go",
    );
    fixture.workspace.update(cx, |workspace, _| {
        workspace.markdown_view = super::DocView::Preview
    });
    fixture.open_file("notes.md", cx);
    let search = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| {
            let doc = &workspace.docs[workspace.active.unwrap()];
            doc.preview
                .as_ref()
                .and_then(|preview| preview.search())
                .map(|search| (search.matches.len(), search.current))
        })
    };

    fixture.press("ctrl-f", cx);
    assert_eq!(search(cx), Some((0, 0)));
    fixture.input("SNAPSHOT", cx);
    // The heading, the bold word, and the list item; Markdown marks are not text.
    assert_eq!(search(cx), Some((3, 0)));
    fixture.press("enter", cx);
    assert_eq!(search(cx), Some((3, 1)));
    fixture.press("shift-enter", cx);
    fixture.press("shift-enter", cx);
    assert_eq!(search(cx), Some((3, 2)));
    fixture.press("escape", cx);
    assert_eq!(search(cx), None);
}

#[gpui_kit::test]
fn a_middle_click_scrolls_toward_the_pointer_until_the_next_click(cx: &mut TestAppContext) {
    use gpui_kit::test::{ClickOptions, TestEventExt as _};

    let long: String = (0..400).map(|n| format!("// line {n}\n")).collect();
    let fixture = Fixture::new(cx, "autoscroll", &[("long.go", long.as_str())], "long.go");
    let editor = fixture.editor(cx);
    let scrolled = |cx: &mut TestAppContext| {
        editor.read_with(cx, |editor, _| -f32::from(editor.scroll_offset().y))
    };
    let target = ("input", editor.entity_id());
    let anchor = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            window.find(target).bounds().center()
        })
        .unwrap();

    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.click_with_options(
            target,
            ClickOptions::new().with_button(gpui_kit::MouseButton::Middle),
            cx,
        );
        window.pointer_move(anchor + gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(120.)), None, cx);
    })
    .unwrap();
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.autoscrolling()));
    // Half a second, a frame at a time.
    for _ in 0..30 {
        cx.executor().advance_clock(std::time::Duration::from_millis(17));
        // The app draws a frame between ticks; the test has to draw it itself.
        fixture.settle(cx);
    }
    fixture.settle(cx);
    let after = scrolled(cx);
    assert!(after > 100., "scrolled {after}px");

    // Any click ends it, and does nothing else.
    let cursor = fixture.cursor(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| window.click(target, cx))
        .unwrap();
    fixture.settle(cx);
    assert!(!fixture.workspace.read_with(cx, |workspace, _| workspace.autoscrolling()));
    assert_eq!(fixture.cursor(cx), cursor);
    for _ in 0..10 {
        cx.executor().advance_clock(std::time::Duration::from_millis(17));
        // The app draws a frame between ticks; the test has to draw it itself.
        fixture.settle(cx);
    }
    assert_eq!(scrolled(cx), after);
}

#[gpui_kit::test]
fn the_compare_view_keeps_both_sides_beside_each_other(cx: &mut TestAppContext) {
    // Forty equal lines, a change, forty more; the new side gains three lines.
    let head: String = (0..40).map(|n| format!("line {n}\n")).collect();
    let tail: String = (40..80).map(|n| format!("line {n}\n")).collect();
    let old = format!("{head}old\n{tail}");
    let new = format!("{head}new\nnew\nnew\nnew\n{tail}");
    let fixture = Fixture::in_repository(cx, "compare", &[("a.txt", old.as_str())], "a.txt");
    std::fs::write(fixture.root.join("a.txt"), &new).unwrap();
    fixture
        .workspace
        .update(cx, |workspace, cx| workspace.refresh_git(cx));
    fixture.settle(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture.workspace.update(cx, |workspace, cx| {
            workspace.open_diff(Some("a.txt".to_string()), window, cx)
        });
    })
    .unwrap();
    let frames = |cx: &mut TestAppContext, n: usize| {
        for _ in 0..n {
            cx.executor().advance_clock(std::time::Duration::from_millis(17));
            fixture.settle(cx);
        }
    };
    frames(cx, 5);
    let (counts, sides) = fixture.workspace.read_with(cx, |workspace, _| {
        let tab = workspace.diff.as_ref().unwrap();
        (tab.counts(), tab.sides())
    });
    assert_eq!(counts, (4, 1));
    let (left, right) = sides.expect("both sides are shown");
    let top_line = |editor: &gpui_kit::Entity<gpui_kit::component::input::EditorState>,
                    cx: &mut TestAppContext| {
        editor.read_with(cx, |editor, _| {
            let height = f32::from(editor.line_height().unwrap());
            (-f32::from(editor.scroll_offset().y) / height).round() as i32
        })
    };
    // It opens at the change, a few lines above it.
    assert_eq!(top_line(&right, cx), 36);

    // Scrolled past the change on the left, the right follows, three lines on.
    let wheel = |cx: &mut TestAppContext, editor: &gpui_kit::Entity<gpui_kit::component::input::EditorState>, lines: f32| {
        use gpui_kit::test::TestWindowExt as _;
        let height = editor.read_with(cx, |editor, _| editor.line_height().unwrap());
        let target = ("input", editor.entity_id());
        cx.update_window(fixture.window.into(), |_, window, cx| {
            window.scroll(
                target,
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), -(height * lines))),
                cx,
            )
        })
        .unwrap();
    };
    wheel(cx, &left, 14.);
    frames(cx, 5);
    assert_eq!(top_line(&left, cx), 50);
    assert_eq!(top_line(&right, cx), 53);

    // And the other way round.
    wheel(cx, &right, -43.);
    frames(cx, 5);
    assert_eq!((top_line(&left, cx), top_line(&right, cx)), (10, 10));
}

#[gpui_kit::test]
fn closing_a_tab_keeps_unsaved_edits(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "close", &[("notes.md", "one\n")], "notes.md");
    fixture.set_cursor(0, 3, cx);
    fixture.input("!", cx);
    assert!(fixture.dirty(cx));

    fixture.press("ctrl-w", cx);

    assert_eq!(fixture.on_disk("notes.md"), "one!\n");
    let open = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.docs.len());
    assert_eq!(open, 0);
}

/// The rows of the open palette as `(path, row, tag)`.
fn palette_rows(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<(String, u32, &'static str)> {
    fixture.workspace.read_with(cx, |workspace, _| {
        let palette = workspace.palette.as_ref().expect("the palette is open");
        assert!(palette.mode == super::palette::Mode::Usages);
        palette
            .hits
            .iter()
            .map(|hit| (hit.path.clone(), hit.row, hit.tag))
            .collect()
    })
}

#[gpui_kit::test]
fn going_to_a_declaration_lists_its_usages(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "usages",
        &[
            GO_MOD,
            ("util.go", "package main\n\nfunc helper() {}\n"),
            ("main.go", "package main\n\nfunc main() {\n\thelper()\n\thelper()\n}\n"),
        ],
        "util.go",
    );
    // Ctrl+B (and Ctrl+click) on the name that is declared here.
    fixture.set_cursor(2, 6, cx);
    fixture.press("ctrl-b", cx);
    assert_eq!(
        palette_rows(&fixture, cx),
        [("main.go".to_string(), 3, ""), ("main.go".to_string(), 4, "")]
    );

    fixture.press("down", cx);
    fixture.press("enter", cx);
    assert_eq!(fixture.active_file(cx), "main.go");
    assert_eq!(fixture.cursor(cx), (4, 1));
}

#[gpui_kit::test]
fn an_interface_lists_its_implementations_first(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "implementations",
        &[
            GO_MOD,
            ("port.go", "package main\n\ntype Wallet interface {\n\tBalance() int\n}\n"),
            (
                "client.go",
                "package main\n\ntype Client struct{}\n\nfunc (Client) Balance() int { return 0 }\n\nfunc use(w Wallet) {}\n",
            ),
        ],
        "port.go",
    );
    fixture.set_cursor(2, 6, cx);
    fixture.press("alt-f7", cx);
    assert_eq!(
        palette_rows(&fixture, cx),
        [("client.go".to_string(), 2, "实现"), ("client.go".to_string(), 6, "")]
    );
}

#[gpui_kit::test]
fn a_single_usage_is_gone_to_directly(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "one-usage",
        &[
            GO_MOD,
            ("util.go", "package main\n\nfunc helper() {}\n"),
            ("main.go", "package main\n\nfunc main() {\n\thelper()\n}\n"),
        ],
        "util.go",
    );
    fixture.set_cursor(2, 6, cx);
    fixture.press("alt-f7", cx);
    assert_eq!(fixture.active_file(cx), "main.go");
    assert_eq!(fixture.cursor(cx), (3, 1));
}

fn console_text(fixture: &Fixture, cx: &mut TestAppContext) -> String {
    fixture.workspace.read_with(cx, |workspace, cx| {
        workspace
            .run
            .console
            .as_ref()
            .expect("something ran")
            .read(cx)
            .text()
    })
}

fn run_phase(fixture: &Fixture, cx: &mut TestAppContext) -> super::run_panel::Phase {
    fixture.workspace.read_with(cx, |workspace, _| workspace.run.phase)
}

const HELLO: &str = "package main\n\nimport \"fmt\"\n\nfunc main() {\n\tfmt.Println(\"hi\")\n}\n";

#[gpui_kit::test]
fn running_without_a_configuration_opens_one_to_fill_in(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "run-config",
        &[GO_MOD, ("cmd/server/main.go", HELLO)],
        "cmd/server/main.go",
    );
    fixture.press("shift-f10", cx);
    let (name, package) = fixture.workspace.read_with(cx, |workspace, _| {
        let editor = workspace.run.editor.as_ref().expect("the editor is open");
        let config = &editor.configs[editor.selected];
        (config.name.clone(), config.package.clone())
    });
    // The open file's package is what a new configuration runs.
    assert_eq!((name.as_str(), package.as_str()), ("server", "./cmd/server"));

    fixture.input("api", cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        fixture
            .workspace
            .update(cx, |workspace, cx| workspace.apply_run_configs(false, window, cx));
    })
    .unwrap();
    fixture.settle(cx);

    let saved = crate::settings::load(&fixture.root).runs;
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "serverapi");
    assert_eq!(saved[0].package, "./cmd/server");
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.run.editor.is_none()));
}

/// A project whose run configuration is saved already.
fn with_config(cx: &mut TestAppContext, name: &str, main: &str) -> Fixture {
    let fixture = Fixture::new(cx, name, &[GO_MOD, ("main.go", main)], "main.go");
    fixture.workspace.update(cx, |workspace, _| {
        workspace.settings.runs = vec![crate::run::RunConfig {
            name: "demo".into(),
            output: "bin/demo.exe".into(),
            ..Default::default()
        }];
    });
    fixture
}

#[gpui_kit::test]
fn building_writes_the_executable_and_says_so(cx: &mut TestAppContext) {
    let fixture = with_config(cx, "build", HELLO);
    fixture.press("ctrl-f9", cx);
    assert_eq!(run_phase(&fixture, cx), super::run_panel::Phase::Built);
    assert!(fixture.root.join("bin/demo.exe").is_file());
    let text = console_text(&fixture, cx);
    assert!(text.contains("$ go build -o"), "{text}");
    assert!(text.contains("编译成功"), "{text}");
}

#[gpui_kit::test]
fn a_failed_build_shows_the_compiler_errors_and_runs_nothing(cx: &mut TestAppContext) {
    let fixture = with_config(cx, "build-fails", "package main\n\nfunc main() {\n\tundefinedName()\n}\n");
    fixture.press("shift-f10", cx);
    assert_eq!(run_phase(&fixture, cx), super::run_panel::Phase::Failed);
    let text = console_text(&fixture, cx);
    assert!(text.contains("main.go:4:2: undefined: undefinedName"), "{text}");
    assert!(text.contains("编译失败"), "{text}");
}

#[gpui_kit::test]
fn a_running_program_can_be_stopped(cx: &mut TestAppContext) {
    let fixture = with_config(cx, "run-stop", HELLO);
    fixture.press("shift-f10", cx);
    assert_eq!(run_phase(&fixture, cx), super::run_panel::Phase::Running);
    assert!(console_text(&fixture, cx).contains("bin"));

    fixture.press("ctrl-f2", cx);
    assert_eq!(run_phase(&fixture, cx), super::run_panel::Phase::Stopped);
    assert!(console_text(&fixture, cx).ends_with("已停止"));
}
