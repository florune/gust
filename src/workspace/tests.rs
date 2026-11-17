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

    // Ctrl+W erases a word in the shell; it must not reach the editor.
    fixture.press("ctrl-w", cx);
    assert_eq!(state(cx), (true, 1, 1));

    fixture.press("ctrl-`", cx);
    assert_eq!(state(cx), (false, 1, 1));
    // Back in the editor, the same key closes the tab again.
    fixture.press("ctrl-f4", cx);
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

    // Closing the tab closes the diff, not the file behind it.
    fixture.press("ctrl-f4", cx);
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

    // With unsaved edits, the editor keeps them and asks which to keep.
    fixture.set_cursor(0, 3, cx);
    fixture.input("!", cx);
    std::fs::write(fixture.root.join("notes.md"), "replaced\n").unwrap();
    changed(cx);
    assert_eq!(fixture.text(cx), "one!\ntwo\nthree\n");
    assert!(fixture.dirty(cx));
    assert_eq!(fixture.conflict(cx).as_deref(), Some("replaced\n"));
}

impl Fixture {
    /// The disk's text of the active file, when it conflicts with the edits.
    fn conflict(&self, cx: &mut TestAppContext) -> Option<String> {
        self.workspace.read_with(cx, |workspace, _| {
            workspace.docs[workspace.active.unwrap()].conflict.clone()
        })
    }

    fn files_changed(&self, cx: &mut TestAppContext) {
        cx.update_window(self.window.into(), |_, window, cx| {
            self.workspace
                .update(cx, |workspace, cx| workspace.files_changed(window, cx));
        })
        .unwrap();
        self.settle(cx);
    }

    fn update(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Workspace, &mut gpui_kit::Window, &mut gpui_kit::Context<Workspace>)) {
        cx.update_window(self.window.into(), |_, window, cx| {
            self.workspace.update(cx, |workspace, cx| f(workspace, window, cx));
        })
        .unwrap();
        self.settle(cx);
    }

    /// Edit the open file, then have something else rewrite it on disk.
    fn make_conflict(&self, disk: &str, cx: &mut TestAppContext) {
        self.set_cursor(0, 3, cx);
        self.input("!", cx);
        std::fs::write(self.root.join("notes.md"), disk).unwrap();
        self.files_changed(cx);
    }
}

#[gpui_kit::test]
fn a_conflict_is_never_saved_over_until_a_side_is_picked(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "conflict-wait", &[("notes.md", "one\ntwo\n")], "notes.md");
    fixture.make_conflict("one\nTWO\n", cx);

    // Saving everything, as switching to the terminal does, leaves it alone;
    // so does closing the tab.
    fixture.update(cx, |workspace, _, cx| workspace.save_all(cx));
    fixture.press("ctrl-f4", cx);
    assert_eq!(std::fs::read_to_string(fixture.root.join("notes.md")).unwrap(), "one\nTWO\n");
    assert_eq!(fixture.tabs(cx), ["notes.md"]);

    // Ctrl+S on purpose keeps the edits.
    fixture.press("ctrl-s", cx);
    assert_eq!(std::fs::read_to_string(fixture.root.join("notes.md")).unwrap(), "one!\ntwo\n");
    assert_eq!(fixture.conflict(cx), None);
}

#[gpui_kit::test]
fn taking_the_disk_version_drops_the_edits(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "conflict-disk", &[("notes.md", "one\ntwo\n")], "notes.md");
    fixture.make_conflict("one\nTWO\n", cx);
    fixture.update(cx, |workspace, window, cx| workspace.take_disk(0, window, cx));
    assert_eq!(fixture.text(cx), "one\nTWO\n");
    assert!(!fixture.dirty(cx));
    assert_eq!(fixture.conflict(cx), None);
}

#[gpui_kit::test]
fn the_disk_compare_takes_changes_one_block_at_a_time(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "conflict-compare",
        &[("notes.md", "one\ntwo\nthree\nfour\n")],
        "notes.md",
    );
    // The disk changed two lines; the editor changed the first. The first
    // two lines make one block, the last line another.
    fixture.make_conflict("one\nTWO\nthree\nFOUR\n", cx);
    fixture.update(cx, |workspace, window, cx| workspace.compare_with_disk(0, window, cx));
    let sides = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, cx| {
            let (left, right) = workspace.diff.as_ref().unwrap().sides().expect("both sides read");
            (left.read(cx).text().to_string(), right.read(cx).text().to_string())
        })
    };
    assert_eq!(
        sides(cx),
        ("one\nTWO\nthree\nFOUR\n".to_string(), "one!\ntwo\nthree\nfour\n".to_string())
    );

    // Take the disk's last block: the edit to the first line stays.
    fixture.update(cx, |workspace, window, cx| workspace.take_disk_chunk(1, window, cx));
    assert_eq!(fixture.text(cx), "one!\ntwo\nthree\nFOUR\n");
    assert_eq!(sides(cx).1, "one!\ntwo\nthree\nFOUR\n");
    assert!(fixture.conflict(cx).is_some());

    // Taking the rest leaves nothing to decide, and the compare closes.
    fixture.update(cx, |workspace, window, cx| workspace.take_disk_chunk(0, window, cx));
    assert_eq!(fixture.text(cx), "one\nTWO\nthree\nFOUR\n");
    assert_eq!(fixture.conflict(cx), None);
    assert!(!fixture.dirty(cx));
}

#[gpui_kit::test]
fn the_outline_lists_the_file_and_starts_at_the_caret(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "outline",
        &[
            GO_MOD,
            (
                "main.go",
                "package main\n\ntype Counter struct {\n\tValue int\n}\n\nfunc (c *Counter) Add(n int) {\n\tc.Value += n\n}\n\nfunc main() {\n}\n",
            ),
        ],
        "main.go",
    );
    fixture.set_cursor(7, 2, cx);
    let symbol = fixture.workspace.read_with(cx, |workspace, cx| workspace.current_symbol(cx));
    assert_eq!(symbol.as_deref(), Some("Counter.Add"));

    fixture.press("ctrl-f12", cx);
    let (rows, selected) = fixture.workspace.read_with(cx, |workspace, _| {
        let palette = workspace.palette.as_ref().expect("the outline is open");
        let rows: Vec<(String, &str)> =
            palette.hits.iter().map(|hit| (hit.preview.clone(), hit.tag)).collect();
        (rows, palette.selected)
    });
    assert_eq!(
        rows,
        [
            ("Counter".to_string(), "类型"),
            ("    Value".to_string(), "字段"),
            ("Counter.Add".to_string(), "方法"),
            ("main".to_string(), "函数"),
        ]
    );
    assert_eq!(selected, 2);

    fixture.input("mai", cx);
    fixture.press("enter", cx);
    assert_eq!(fixture.cursor(cx), (10, 5));
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

    fixture.press("ctrl-f4", cx);

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
            ("port.go", "package main\n\ntype Store interface {\n\tLoad() int\n}\n"),
            (
                "client.go",
                "package main\n\ntype Client struct{}\n\nfunc (Client) Load() int { return 0 }\n\nfunc use(s Store) {}\n",
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
    // The prompt names the folder the build runs in.
    let folder = fixture.root.file_name().unwrap().to_string_lossy().into_owned();
    assert!(text.contains(&format!("{folder}> go build -o")), "{text}");
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

#[gpui_kit::test]
fn the_caret_line_says_who_changed_it_last(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "blame", &[("notes.md", "one\ntwo\n")], "notes.md");
    let note = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, cx| {
            workspace.blame_note(0, cx).map(|note| note.to_string())
        })
    };
    // The git status arrives in the background, then the blame.
    fixture.settle(cx);
    fixture.settle(cx);
    let committed = note(cx).expect("the line is blamed");
    assert!(committed.starts_with("Test, "), "{committed}");
    assert!(committed.ends_with("· first"), "{committed}");

    // An edited line has no commit yet; the blame is read again once typing pauses.
    fixture.set_cursor(1, 3, cx);
    fixture.input("!", cx);
    cx.executor().advance_clock(std::time::Duration::from_millis(600));
    fixture.settle(cx);
    assert_eq!(note(cx).as_deref(), Some("未提交的修改"));
    fixture.set_cursor(0, 0, cx);
    assert!(note(cx).unwrap().starts_with("Test, "));
}

#[gpui_kit::test]
fn a_split_shows_the_same_file_twice_and_keeps_both_in_step(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "split", &[("a.md", "one\n"), ("b.md", "bee\n")], "a.md");
    fixture.press("ctrl-b", cx); // nothing to go to; the editor keeps the keys
    fixture.update(cx, |workspace, window, cx| workspace.on_split_right(&super::split::SplitRight, window, cx));
    let right = fixture.workspace.read_with(cx, |workspace, _| {
        workspace.split.as_ref().expect("split").editor.clone()
    });

    // Typing on the right edits the file.
    fixture.input("1", cx);
    assert_eq!(right.read_with(cx, |editor, _| editor.text().to_string()), "1one\n");
    assert_eq!(fixture.text(cx), "1one\n");
    assert!(fixture.dirty(cx));

    // A file opened now goes to the right pane; the left keeps its file.
    fixture.open_file("b.md", cx);
    let (left, right_path) = fixture.workspace.read_with(cx, |workspace, _| {
        let left = workspace.docs[workspace.active.unwrap()].title.to_string();
        let right = workspace.split.as_ref().unwrap().path.clone();
        (left, right)
    });
    assert_eq!(left, "a.md");
    assert_eq!(right_path, fixture.root.join("b.md"));

    // Closing a tab in the right pane closes the pane, not a tab.
    fixture.press("ctrl-f4", cx);
    let state = fixture
        .workspace
        .read_with(cx, |workspace, _| (workspace.split.is_some(), workspace.docs.len()));
    assert_eq!(state, (false, 2));
}

#[gpui_kit::test]
fn editing_the_left_pane_shows_on_the_right(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "split-left", &[("a.md", "one\n")], "a.md");
    fixture.update(cx, |workspace, window, cx| {
        let path = workspace.docs[0].path.clone();
        workspace.split_right(&path, window, cx);
        // Back to the left pane.
        workspace.activate(0, window, cx);
    });
    assert!(!fixture.workspace.read_with(cx, |workspace, _| workspace.split_focused()));
    fixture.set_cursor(0, 3, cx);
    fixture.input("!", cx);
    let right = fixture.workspace.read_with(cx, |workspace, cx| {
        workspace.split.as_ref().unwrap().editor.read(cx).text().to_string()
    });
    assert_eq!(right, "one!\n");

    // Closing the file's tab closes the pane that shows it.
    fixture.press("ctrl-f4", cx);
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.split.is_none()));
}

impl Fixture {
    /// The middle of the text at byte `range` of the active editor, on screen.
    fn point_of(&self, range: std::ops::Range<usize>, cx: &mut TestAppContext) -> gpui_kit::Point<gpui_kit::Pixels> {
        self.settle(cx);
        let bounds = self
            .editor(cx)
            .read_with(cx, |editor, _| editor.range_to_bounds(&range))
            .expect("the text is laid out");
        bounds.center()
    }

    fn mouse(&self, event: gpui_kit::PlatformInput, cx: &mut TestAppContext) {
        cx.update_window(self.window.into(), |_, window, cx| {
            window.dispatch_event(event, cx);
        })
        .unwrap();
        self.settle(cx);
    }

    /// Press and release the left button at `at` with `modifiers`, optionally
    /// dragging to `to` in between.
    fn click(
        &self,
        at: gpui_kit::Point<gpui_kit::Pixels>,
        to: Option<gpui_kit::Point<gpui_kit::Pixels>>,
        modifiers: gpui_kit::Modifiers,
        cx: &mut TestAppContext,
    ) {
        use gpui_kit::{MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PlatformInput};
        self.mouse(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: at,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        let end = to.unwrap_or(at);
        if to.is_some() {
            self.mouse(
                PlatformInput::MouseMove(MouseMoveEvent {
                    position: end,
                    pressed_button: Some(MouseButton::Left),
                    modifiers,
                }),
                cx,
            );
        }
        self.mouse(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: end,
                modifiers,
                click_count: 1,
            }),
            cx,
        );
    }
}

fn definition_project(cx: &mut TestAppContext, name: &str) -> Fixture {
    Fixture::new(
        cx,
        name,
        &[
            GO_MOD,
            ("util.go", "package main\n\n// helper helps.\nfunc helper() {}\n"),
            ("main.go", "package main\n\nfunc main() {\n\thelper()\n}\n"),
        ],
        "main.go",
    )
}

#[gpui_kit::test]
fn ctrl_click_goes_to_the_definition_without_hovering_first(cx: &mut TestAppContext) {
    let fixture = definition_project(cx, "ctrl-click");
    let source = fixture.text(cx);
    let at = source.find("helper()").unwrap();
    let point = fixture.point_of(at + 2..at + 3, cx);
    fixture.click(point, None, gpui_kit::Modifiers::control(), cx);
    assert_eq!(fixture.active_file(cx), "util.go");
    assert_eq!(fixture.cursor(cx), (3, 5));
}

#[gpui_kit::test]
fn back_and_forward_return_across_a_jump(cx: &mut TestAppContext) {
    let fixture = definition_project(cx, "back");
    fixture.set_cursor(3, 3, cx);
    fixture.press("ctrl-b", cx);
    assert_eq!(fixture.active_file(cx), "util.go");

    fixture.press("ctrl-alt-left", cx);
    assert_eq!(fixture.active_file(cx), "main.go");
    assert_eq!(fixture.cursor(cx), (3, 3));

    fixture.press("ctrl-alt-right", cx);
    assert_eq!(fixture.active_file(cx), "util.go");
    assert_eq!(fixture.cursor(cx), (3, 5));
}

#[gpui_kit::test]
fn alt_drag_selects_a_block_and_alt_shift_click_adds_a_caret(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "alt-mouse", &[("a.txt", "abcd\nefgh\nijkl\n")], "a.txt");
    // Alt+drag from before "b" to after "f": the first column of two lines.
    let from = fixture.point_of(1..1, cx);
    let to = fixture.point_of(7..7, cx);
    fixture.click(from, Some(to), gpui_kit::Modifiers::alt(), cx);
    fixture.input("X", cx);
    assert_eq!(fixture.text(cx), "aXcd\neXgh\nijkl\n");

    // Alt+Shift+click puts a second caret where it clicks.
    fixture.set_cursor(2, 0, cx);
    let other = fixture.point_of(4..4, cx);
    let mut alt_shift = gpui_kit::Modifiers::alt();
    alt_shift.shift = true;
    fixture.click(other, None, alt_shift, cx);
    fixture.input("!", cx);
    assert_eq!(fixture.text(cx), "aXcd!\neXgh\n!ijkl\n");
}

#[gpui_kit::test]
fn ctrl_w_grows_the_selection_and_ctrl_shift_w_shrinks_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "extend",
        &[GO_MOD, ("main.go", "package main

func main() {
	println(1, 22)
}
")],
        "main.go",
    );
    let selected = |cx: &mut TestAppContext| {
        fixture.editor(cx).read_with(cx, |editor, _| editor.selected_text().to_string())
    };
    fixture.set_cursor(3, 13, cx);
    fixture.press("ctrl-w", cx);
    assert_eq!(selected(cx), "22");
    fixture.press("ctrl-w", cx);
    assert_eq!(selected(cx), "1, 22");
    fixture.press("ctrl-w", cx);
    assert_eq!(selected(cx), "(1, 22)");
    fixture.press("ctrl-shift-w", cx);
    assert_eq!(selected(cx), "1, 22");
    fixture.press("ctrl-shift-w", cx);
    assert_eq!(selected(cx), "22");
    // The tab is still there: Ctrl+W no longer closes it.
    assert_eq!(fixture.tabs(cx), ["main.go"]);
}

impl Fixture {
    fn modifiers(&self, modifiers: gpui_kit::Modifiers, cx: &mut TestAppContext) {
        self.mouse(
            gpui_kit::PlatformInput::ModifiersChanged(gpui_kit::ModifiersChangedEvent {
                modifiers,
                capslock: Default::default(),
            }),
            cx,
        );
    }

    fn tap_shift(&self, cx: &mut TestAppContext) {
        self.modifiers(gpui_kit::Modifiers::shift(), cx);
        self.modifiers(gpui_kit::Modifiers::default(), cx);
    }
}

#[gpui_kit::test]
fn double_shift_opens_the_file_search_and_a_line_opens_at_it(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "double-shift",
        &[
            ("internal/queue/worker.go", "package queue\n\nfunc a() {}\n\nfunc b() {}\n"),
            ("internal/cache/worker.go", "package cache\n"),
            ("main.go", "package main\n"),
        ],
        "main.go",
    );
    // One tap does nothing; a key between taps cancels them.
    fixture.tap_shift(cx);
    fixture.press("a", cx);
    fixture.tap_shift(cx);
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.palette.is_none()));

    fixture.tap_shift(cx);
    fixture.tap_shift(cx);
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.palette.is_some()));

    fixture.input("queue/worker.go:5", cx);
    fixture.press("enter", cx);
    assert_eq!(fixture.active_file(cx), "internal/queue/worker.go");
    assert_eq!(fixture.cursor(cx), (4, 0));
}


#[gpui_kit::test]
fn files_are_made_renamed_and_deleted_from_the_tree(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "file-ops", &[GO_MOD, ("main.go", "package main\n")], "main.go");
    let root = fixture.root.clone();
    let prompt = |action: super::file_ops::FileAction, cx: &mut TestAppContext| {
        fixture.update(cx, |workspace, window, cx| workspace.open_file_prompt(action, window, cx));
    };

    // A new Go file in a new folder starts with that folder's package.
    prompt(super::file_ops::FileAction::New { dir: root.clone(), folder: false }, cx);
    fixture.input("api/h.go", cx);
    fixture.press("enter", cx);
    assert_eq!(std::fs::read_to_string(root.join("api/h.go")).unwrap(), "package api\n\n");
    assert_eq!(fixture.active_file(cx), "api/h.go");

    // Renaming starts with the name selected without its extension.
    prompt(super::file_ops::FileAction::Rename(root.join("api/h.go")), cx);
    fixture.input("handler", cx);
    fixture.press("enter", cx);
    assert!(root.join("api/handler.go").is_file() && !root.join("api/h.go").exists());
    assert_eq!(fixture.active_file(cx), "api/handler.go");

    // A name that is taken is refused, and the box says why.
    prompt(super::file_ops::FileAction::New { dir: root.clone(), folder: false }, cx);
    fixture.input("main.go", cx);
    fixture.press("enter", cx);
    let error = fixture.workspace.read_with(cx, |workspace, _| {
        workspace.file_prompt.as_ref().and_then(|prompt| prompt.error.clone())
    });
    assert!(error.is_some());
    fixture.press("escape", cx);

    // Deleting a folder closes what was open from it.
    prompt(super::file_ops::FileAction::Delete(root.join("api")), cx);
    fixture.press("enter", cx);
    assert!(!root.join("api").exists());
    assert_eq!(fixture.tabs(cx), ["main.go"]);
}

#[gpui_kit::test]
fn goland_editing_keys_work_on_lines(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "editing-keys",
        &[GO_MOD, ("main.go", "package main\n\nfunc main() {\n\ta()\n\tb()\n}\n")],
        "main.go",
    );
    fixture.set_cursor(3, 1, cx);
    fixture.press("ctrl-/", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\t// a()\n\tb()\n}\n");
    // The caret went on to the next line; comment it back in from there.
    fixture.set_cursor(3, 1, cx);
    fixture.press("ctrl-/", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\ta()\n\tb()\n}\n");

    fixture.set_cursor(3, 1, cx);
    fixture.press("ctrl-d", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\ta()\n\ta()\n\tb()\n}\n");
    fixture.press("ctrl-y", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\ta()\n\tb()\n}\n");

    fixture.set_cursor(4, 1, cx);
    fixture.press("ctrl-shift-up", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\tb()\n\ta()\n}\n");
    assert_eq!(fixture.cursor(cx), (3, 1));

    // Undo takes the move back; Ctrl+Shift+Z puts it in again.
    fixture.press("ctrl-z", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\ta()\n\tb()\n}\n");
    fixture.press("ctrl-shift-z", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\tb()\n\ta()\n}\n");
}

#[gpui_kit::test]
fn ctrl_e_lists_the_files_shown_last_and_picks_the_one_before(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "recent-files",
        &[("a.md", "a\n"), ("b.md", "b\n"), ("c.md", "c\n")],
        "a.md",
    );
    fixture.open_file("b.md", cx);
    fixture.open_file("c.md", cx);
    fixture.press("ctrl-e", cx);
    let rows = fixture.workspace.read_with(cx, |workspace, _| {
        let palette = workspace.palette.as_ref().unwrap();
        let rows: Vec<String> = palette.hits.iter().map(|hit| hit.path.clone()).collect();
        (rows, palette.selected)
    });
    assert_eq!(rows, (vec!["c.md".into(), "b.md".into(), "a.md".into()], 1));
    fixture.press("enter", cx);
    assert_eq!(fixture.active_file(cx), "b.md");
}

#[gpui_kit::test]
fn compile_errors_show_in_the_editor_and_f2_goes_to_them(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "problems",
        &[
            GO_MOD,
            (
                "main.go",
                "package main\n\nfunc main() {\n\tprintln(1)\n\tprintln(missing)\n}\n",
            ),
        ],
        "main.go",
    );
    // The check runs once things settle after opening.
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    fixture.settle(cx);
    let counts = fixture.workspace.read_with(cx, |workspace, _| workspace.problems.counts());
    assert_eq!(counts, (1, 0));
    let underlined = fixture
        .editor(cx)
        .read_with(cx, |editor, _| editor.diagnostics().map_or(0, |set| set.len()));
    assert_eq!(underlined, 1);

    fixture.set_cursor(0, 0, cx);
    fixture.press("f2", cx);
    assert_eq!(fixture.cursor(cx), (4, 9));

    // Fixed and saved: the problem goes away.
    fixture.set_cursor(4, 9, cx);
    for _ in 0.."missing".len() {
        fixture.press("delete", cx);
    }
    fixture.input("2", cx);
    fixture.press("ctrl-s", cx);
    cx.executor().advance_clock(std::time::Duration::from_millis(400));
    fixture.settle(cx);
    let counts = fixture.workspace.read_with(cx, |workspace, _| workspace.problems.counts());
    assert_eq!(counts, (0, 0));
}

#[gpui_kit::test]
fn a_changed_block_rolls_back_from_the_keyboard_or_its_mark(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "rollback-lines", &[("notes.md", "one\ntwo\nthree\n")], "notes.md");
    fixture.settle(cx);
    fixture.set_cursor(1, 3, cx);
    fixture.input("!", cx);
    assert_eq!(fixture.text(cx), "one\ntwo!\nthree\n");

    fixture.press("ctrl-alt-z", cx);
    assert_eq!(fixture.text(cx), "one\ntwo\nthree\n");

    // Again, by a click on the mark beside the changed line.
    fixture.set_cursor(2, 5, cx);
    fixture.input("?", cx);
    let (input, top, height) = fixture.editor(cx).read_with(cx, |editor, _| {
        (editor.input_bounds(), editor.text_bounds().unwrap().top(), editor.line_height().unwrap())
    });
    let mark = gpui_kit::point(input.left() + gpui_kit::px(2.), top + height * 2.5);
    fixture.mouse(
        gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
            button: gpui_kit::MouseButton::Left,
            position: mark,
            click_count: 1,
            ..Default::default()
        }),
        cx,
    );
    let shown = fixture.workspace.read_with(cx, |workspace, _| {
        workspace.hunk.as_ref().map(|hunk| hunk.old.clone())
    });
    assert_eq!(shown.as_deref(), Some("three\n"));
    fixture.update(cx, |workspace, window, cx| {
        let chunk = workspace.hunk.as_ref().unwrap().chunk.clone();
        workspace.roll_back_for_test(&chunk, window, cx);
    });
    assert_eq!(fixture.text(cx), "one\ntwo\nthree\n");
}

#[gpui_kit::test]
fn a_whole_file_rolls_back_from_the_commit_panel(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "rollback-file", &[("notes.md", "one\n")], "notes.md");
    std::fs::write(fixture.root.join("notes.md"), "changed\n").unwrap();
    std::fs::write(fixture.root.join("new.md"), "new\n").unwrap();
    fixture.files_changed(cx);
    fixture.settle(cx);

    for path in ["notes.md", "new.md"] {
        fixture.update(cx, |workspace, window, cx| workspace.ask_rollback_file(path.into(), window, cx));
        fixture.press("enter", cx);
    }
    assert_eq!(std::fs::read_to_string(fixture.root.join("notes.md")).unwrap(), "one\n");
    assert!(!fixture.root.join("new.md").exists());
    assert_eq!(fixture.text(cx), "one\n");
    assert!(!fixture.dirty(cx));
}

#[gpui_kit::test]
fn a_project_reopens_with_its_tabs_and_carets(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "session",
        &[("a.md", "one\ntwo\nthree\n"), ("b.md", "bee\n")],
        "a.md",
    );
    fixture.set_cursor(2, 3, cx);
    fixture.open_file("b.md", cx);

    let other = std::env::temp_dir().join(format!("gust-ui-session-other-{}", std::process::id()));
    std::fs::create_dir_all(&other).unwrap();
    let root = fixture.root.clone();
    fixture.update(cx, |workspace, window, cx| workspace.set_root(other.clone(), window, cx));
    assert!(fixture.tabs(cx).is_empty());

    fixture.update(cx, |workspace, window, cx| workspace.set_root(root.clone(), window, cx));
    assert_eq!(fixture.tabs(cx), ["a.md", "b.md"]);
    assert_eq!(fixture.active_file(cx), "b.md");
    fixture.press("ctrl-e", cx);
    fixture.press("enter", cx);
    assert_eq!(fixture.active_file(cx), "a.md");
    assert_eq!(fixture.cursor(cx), (2, 3));
    let _ = std::fs::remove_dir_all(&other);
}

#[gpui_kit::test]
fn another_project_opens_here_or_in_a_new_window(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "open-choice", &[("a.md", "a\n")], "a.md");
    let other = std::env::temp_dir().join(format!("gust-ui-open-choice-other-{}", std::process::id()));
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("b.md"), "b\n").unwrap();

    // A new window leaves this one as it was.
    fixture.update(cx, |workspace, window, cx| workspace.choose_project(other.clone(), window, cx));
    fixture.update(cx, |workspace, window, cx| {
        workspace.on_open_in_new_window(&super::windows::OpenInNewWindow, window, cx)
    });
    assert_eq!(cx.windows().len(), 2);
    assert_eq!(fixture.tabs(cx), ["a.md"]);

    // Enter opens it here.
    fixture.update(cx, |workspace, window, cx| workspace.choose_project(other.clone(), window, cx));
    fixture.press("enter", cx);
    let root = fixture.workspace.read_with(cx, |workspace, _| workspace.root.clone());
    assert_eq!(root, other);
    let _ = std::fs::remove_dir_all(&other);
}

/// The rows of the open palette, in whatever mode it is.
fn rows(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<String> {
    fixture.workspace.read_with(cx, |workspace, _| {
        let palette = workspace.palette.as_ref().expect("the palette is open");
        palette.hits.iter().map(|hit| hit.preview.clone()).collect()
    })
}

const STORE: &str = "package store\n\n// Store keeps items.\ntype Store struct{}\n\n// Load returns an item.\nfunc (s *Store) Load(id string) int {\n\treturn 0\n}\n";

#[gpui_kit::test]
fn ctrl_n_finds_a_declaration_anywhere_in_the_project(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "find-symbol",
        &[GO_MOD, ("store/store.go", STORE), ("main.go", "package main\n\nfunc main() {}\n")],
        "main.go",
    );
    fixture.press("ctrl-n", cx);
    fixture.input("store.lo", cx);
    assert_eq!(rows(&fixture, cx), ["Store.Load"]);
    fixture.press("enter", cx);
    assert_eq!(fixture.active_file(cx), "store/store.go");
    assert_eq!(fixture.cursor(cx), (6, 16));
}

#[gpui_kit::test]
fn ctrl_q_shows_the_declaration_and_its_comment(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "quick-doc",
        &[
            GO_MOD,
            ("store/store.go", STORE),
            (
                "main.go",
                "package main\n\nimport \"example.com/demo/store\"\n\nfunc main() {\n\tvar s store.Store\n\ts.Load(\"a\")\n}\n",
            ),
        ],
        "main.go",
    );
    fixture.set_cursor(6, 4, cx);
    fixture.press("ctrl-q", cx);
    let shown = fixture.workspace.read_with(cx, |workspace, _| {
        workspace
            .quick_doc
            .as_ref()
            .map(|quick| (quick.doc.header.clone(), quick.doc.comment.clone()))
    });
    assert_eq!(
        shown,
        Some(("func (s *Store) Load(id string) int".to_string(), "Load returns an item.".to_string()))
    );
    // Any key closes it.
    fixture.press("escape", cx);
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.quick_doc.is_none()));
}

#[gpui_kit::test]
fn a_test_function_runs_from_the_caret_or_its_marker(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "run-test",
        &[
            GO_MOD,
            ("store/store.go", STORE),
            (
                "store/store_test.go",
                "package store\n\nimport \"testing\"\n\nfunc TestLoad(t *testing.T) {\n\t_ = (&Store{}).Load(\"a\")\n}\n",
            ),
        ],
        "store/store_test.go",
    );
    // A run marker sits beside the test function.
    let markers = fixture.workspace.update(cx, |workspace, cx| {
        let ix = workspace.active.unwrap();
        workspace.render_test_markers(ix, cx).is_some()
    });
    assert!(markers);

    fixture.set_cursor(5, 2, cx);
    fixture.press("ctrl-shift-f10", cx);
    assert_eq!(run_phase(&fixture, cx), super::run_panel::Phase::Running);
    assert!(console_text(&fixture, cx).contains("go test -v -count=1 -run ^TestLoad$ ./store"));

    // Outside a test function: the package's tests.
    fixture.set_cursor(0, 0, cx);
    fixture.press("ctrl-shift-f10", cx);
    let text = console_text(&fixture, cx);
    assert!(text.contains("go test -v -count=1 ./store") && !text.contains("-run ^"), "{text}");
}

#[gpui_kit::test]
fn shift_f6_renames_a_symbol_in_open_and_closed_files(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "rename-symbol",
        &[
            GO_MOD,
            ("store/store.go", STORE),
            (
                "main.go",
                "package main\n\nimport \"example.com/demo/store\"\n\nfunc main() {\n\tvar s store.Store\n\ts.Load(\"a\")\n\ts.Load(\"b\")\n}\n",
            ),
        ],
        "main.go",
    );
    fixture.set_cursor(6, 4, cx);
    fixture.press("shift-f6", cx);
    let detail = fixture.workspace.read_with(cx, |workspace, _| {
        workspace.file_prompt.as_ref().map(|prompt| match &prompt.action {
            super::file_ops::FileAction::RenameSymbol(plan) => (plan.count(), plan.places.len()),
            _ => (0, 0),
        })
    });
    assert_eq!(detail, Some((3, 2)));

    // The name is selected: typing replaces it.
    fixture.input("Fetch", cx);
    fixture.press("enter", cx);
    assert_eq!(
        fixture.text(cx),
        "package main\n\nimport \"example.com/demo/store\"\n\nfunc main() {\n\tvar s store.Store\n\ts.Fetch(\"a\")\n\ts.Fetch(\"b\")\n}\n"
    );
    let store = std::fs::read_to_string(fixture.root.join("store/store.go")).unwrap();
    assert!(store.contains("func (s *Store) Fetch(id string) int"), "{store}");
}

#[gpui_kit::test]
fn ctrl_shift_r_replaces_text_in_every_file(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "replace",
        &[("a.md", "old one\n"), ("docs/b.md", "Old two old\n"), ("c.md", "nothing\n")],
        "a.md",
    );
    fixture.press("ctrl-shift-r", cx);
    fixture.input("old", cx);
    fixture.update(cx, |workspace, window, cx| {
        let replacement = workspace.palette.as_ref().unwrap().replacement.clone().unwrap();
        replacement.update(cx, |input, cx| input.set_value("new", window, cx));
        workspace.replace_in_files(window, cx);
    });
    assert_eq!(fixture.text(cx), "new one\n");
    assert_eq!(std::fs::read_to_string(fixture.root.join("docs/b.md")).unwrap(), "new two new\n");
    assert_eq!(std::fs::read_to_string(fixture.root.join("c.md")).unwrap(), "nothing\n");
    let notice = fixture.workspace.read_with(cx, |workspace, _| workspace.info.clone());
    assert_eq!(notice.as_deref(), Some("已替换 3 处，2 个文件"));
}

#[gpui_kit::test]
fn local_history_keeps_saved_versions_to_compare_and_restore(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "local-history", &[("a.md", "one\n")], "a.md");
    fixture.set_cursor(0, 3, cx);
    fixture.input("!", cx);
    fixture.press("ctrl-s", cx);
    fixture.input("?", cx);
    fixture.press("ctrl-s", cx);
    assert_eq!(fixture.text(cx), "one!?\n");

    let path = fixture.root.join("a.md");
    fixture.update(cx, |workspace, window, cx| workspace.show_local_history(path.clone(), window, cx));
    assert_eq!(rows(&fixture, cx).len(), 2);

    // The oldest version: what the file was before the first save.
    fixture.press("down", cx);
    fixture.press("enter", cx);
    let sides = fixture.workspace.read_with(cx, |workspace, cx| {
        let (left, right) = workspace.diff.as_ref().unwrap().sides().expect("both sides read");
        (left.read(cx).text().to_string(), right.read(cx).text().to_string())
    });
    assert_eq!(sides, ("one\n".to_string(), "one!?\n".to_string()));

    fixture.update(cx, |workspace, window, cx| {
        let version = match &workspace.diff.as_ref().unwrap().source {
            super::compare_view::Source::Version { version, .. } => version.clone(),
            _ => unreachable!(),
        };
        workspace.restore_version(&path, &version, window, cx);
    });
    assert_eq!(fixture.text(cx), "one\n");
}

fn palette_mode(fixture: &Fixture, cx: &mut TestAppContext) -> Option<super::palette::Mode> {
    fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.palette.as_ref().map(|palette| palette.mode))
}

#[gpui_kit::test]
fn settings_change_the_font_size_and_what_saving_does(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "settings", &[GO_MOD, ("main.go", "package main\n")], "main.go");
    let size = |cx: &mut TestAppContext| {
        cx.update(|cx| f32::from(gpui_kit::component::ActiveTheme::theme(cx).mono_font_size))
    };
    fixture.update(cx, |workspace, _, cx| {
        workspace.change_app_settings(cx, |app| {
            app.font_size = 16.;
            app.format_on_save = false;
        })
    });
    assert_eq!(size(cx), 16.);

    // Ctrl+wheel over the editor makes the code larger.
    let over = fixture.point_of(0..1, cx);
    fixture.mouse(
        gpui_kit::PlatformInput::Scroll(gpui_kit::ScrollEvent {
            position: over,
            delta: gpui_kit::ScrollDelta::Lines(gpui_kit::point(0., 3.)),
            modifiers: gpui_kit::Modifiers::control(),
            ..Default::default()
        }),
        cx,
    );
    assert_eq!(size(cx), 17.);

    // Saving writes the text as it is.
    fixture.set_cursor(0, 12, cx);
    fixture.input("\nfunc  main( ) {}", cx);
    fixture.press("ctrl-s", cx);
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("main.go")).unwrap(),
        "package main\nfunc  main( ) {}\n"
    );
}

#[gpui_kit::test]
fn amend_stash_and_a_new_branch_from_the_commit_panel(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "git-more", &[("notes.md", "one\n")], "notes.md");
    fixture.settle(cx);
    let message = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, cx| workspace.commit_message.read(cx).value().to_string())
    };
    fixture.update(cx, |workspace, window, cx| workspace.set_amend_for_test(true, window, cx));
    assert_eq!(message(cx), "first");

    // Stash takes the edit away; bringing it back restores it.
    std::fs::write(fixture.root.join("notes.md"), "changed\n").unwrap();
    fixture.files_changed(cx);
    fixture.update(cx, |workspace, window, cx| {
        workspace.open_file_prompt(super::file_ops::FileAction::Stash, window, cx)
    });
    fixture.press("enter", cx);
    fixture.files_changed(cx);
    assert_eq!(std::fs::read_to_string(fixture.root.join("notes.md")).unwrap(), "one\n");
    fixture.update(cx, |workspace, window, cx| workspace.show_stashes(window, cx));
    assert_eq!(palette_mode(&fixture, cx), Some(super::palette::Mode::Stashes));
    fixture.press("enter", cx);
    assert_eq!(std::fs::read_to_string(fixture.root.join("notes.md")).unwrap(), "changed\n");

    fixture.update(cx, |workspace, window, cx| {
        workspace.open_file_prompt(super::file_ops::FileAction::NewBranch, window, cx)
    });
    fixture.input("feature/x", cx);
    fixture.press("enter", cx);
    fixture.update(cx, |workspace, _, cx| workspace.refresh_git(cx));
    let branch = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.git.as_ref().map(|status| status.branch.clone()));
    assert_eq!(branch.as_deref(), Some("feature/x"));
}

#[gpui_kit::test]
fn alt_enter_adds_json_tags_and_implements_an_interface(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "intentions",
        &[
            GO_MOD,
            ("port/port.go", "package port\n\ntype Namer interface {\n\tName() string\n}\n"),
            ("main.go", "package main\n\ntype Config struct {\n\tKey string\n}\n"),
        ],
        "main.go",
    );
    fixture.set_cursor(3, 2, cx);
    fixture.press("alt-enter", cx);
    assert_eq!(rows(&fixture, cx), ["添加 json 标签", "为 Config 实现接口…", "运行 go mod tidy"]);
    fixture.press("enter", cx);
    assert!(fixture.text(cx).contains("Key string `json:\"key\"`"), "{}", fixture.text(cx));

    fixture.set_cursor(2, 6, cx);
    fixture.press("alt-enter", cx);
    fixture.input("实现", cx);
    fixture.press("enter", cx);
    assert_eq!(palette_mode(&fixture, cx), Some(super::palette::Mode::Interfaces));
    fixture.press("enter", cx);
    assert!(
        fixture.text(cx).contains("func (c *Config) Name() string {\n\tpanic(\"not implemented\")\n}"),
        "{}",
        fixture.text(cx)
    );
}

#[gpui_kit::test]
fn tab_after_iferr_expands_the_template(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "templates",
        &[GO_MOD, ("main.go", "package main\n\nfunc load() (int, error) {\n\t\n}\n")],
        "main.go",
    );
    fixture.set_cursor(3, 1, cx);
    fixture.input("iferr", cx);
    fixture.press("escape", cx);
    fixture.press("tab", cx);
    assert_eq!(
        fixture.text(cx),
        "package main\n\nfunc load() (int, error) {\n\tif err != nil {\n\t\treturn 0, err\n\t}\n}\n"
    );
    assert_eq!(fixture.cursor(cx), (4, 15));
}

#[gpui_kit::test]
fn ctrl_shift_a_runs_a_command_by_name(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "find-action",
        &[GO_MOD, ("main.go", "package main\n\nfunc main() {}\n")],
        "main.go",
    );
    fixture.press("ctrl-shift-a", cx);
    fixture.input("文件结构", cx);
    assert_eq!(rows(&fixture, cx), ["文件结构"]);
    fixture.press("enter", cx);
    assert_eq!(palette_mode(&fixture, cx), Some(super::palette::Mode::Outline));
}

#[gpui_kit::test]
fn tab_still_indents_where_no_template_applies(cx: &mut TestAppContext) {
    let fixture = Fixture::new(
        cx,
        "plain-tab",
        &[GO_MOD, ("main.go", "package main\n\nfunc main() {\nx\n}\n")],
        "main.go",
    );
    fixture.set_cursor(3, 0, cx);
    fixture.press("tab", cx);
    assert_eq!(fixture.text(cx), "package main\n\nfunc main() {\n\tx\n}\n");
}

#[gpui_kit::test]
fn conflicts_are_resolved_from_the_commit_panel_one_file_at_a_time(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(
        cx,
        "merge-dialog",
        &[("a.txt", "top\none\nm1\nm2\nm3\nm4\ntwo\nend\n"), ("b.txt", "b\n"), ("other.md", "x\n")],
        "other.md",
    );
    let root = fixture.root.clone();
    git(&root, &["switch", "--quiet", "-c", "feature"]);
    std::fs::write(root.join("a.txt"), "top\nONE-feature\nm1\nm2\nm3\nm4\ntwo-feature\nend\n").unwrap();
    std::fs::write(root.join("b.txt"), "b-feature\n").unwrap();
    git(&root, &["commit", "--quiet", "-am", "feature"]);
    git(&root, &["switch", "--quiet", "main"]);
    std::fs::write(root.join("a.txt"), "top\nONE-main\nm1\nm2\nm3\nm4\ntwo-main\nend\n").unwrap();
    std::fs::write(root.join("b.txt"), "b-main\n").unwrap();
    git(&root, &["commit", "--quiet", "-am", "main"]);
    let merged = crate::proc::command("git").args(["merge", "feature"]).current_dir(&root).output().unwrap();
    assert!(!merged.status.success());
    fixture.files_changed(cx);
    fixture.settle(cx);
    let conflicted = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| workspace.conflicted_paths())
    };
    assert_eq!(conflicted(cx), ["a.txt", "b.txt"]);

    // The commit panel opens the list; a.txt opens in the merge window.
    fixture.update(cx, |workspace, window, cx| {
        workspace.open_merge_dialog(window, cx);
        workspace.open_merge("a.txt".into(), cx);
    });
    let (rows, conflicts) = fixture.workspace.read_with(cx, |workspace, _| {
        let merge = workspace.merge_dialog.as_ref().unwrap().merge.as_ref().unwrap();
        (merge.rows.len(), merge.decisions.len())
    });
    assert_eq!((rows, conflicts), (8, 2));

    // First conflict: keep the left (incoming), drop the right; second: keep both.
    use super::merge_view::Decision::{Drop, Keep};
    fixture.update(cx, |workspace, window, cx| {
        workspace.decide(0, true, Keep, cx);
        workspace.decide(0, false, Drop, cx);
        workspace.decide(1, true, Keep, cx);
        workspace.decide(1, false, Keep, cx);
        workspace.apply_merge(window, cx);
    });
    fixture.settle(cx);
    assert_eq!(
        std::fs::read_to_string(root.join("a.txt")).unwrap(),
        "top\nONE-feature\nm1\nm2\nm3\nm4\ntwo-main\ntwo-feature\nend\n"
    );

    // b.txt takes the incoming side whole.
    fixture.update(cx, |workspace, window, cx| workspace.take_side("b.txt".into(), false, window, cx));
    fixture.settle(cx);
    fixture.update(cx, |workspace, _, cx| workspace.refresh_git(cx));
    assert!(conflicted(cx).is_empty());
    assert_eq!(std::fs::read_to_string(root.join("b.txt")).unwrap(), "b-feature\n");
}

#[gpui_kit::test]
fn the_keymap_page_rebinds_a_command(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "keymap", &[GO_MOD, ("main.go", "package main\n")], "main.go");

    // The defaults the page lists are the keys actually bound.
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        for command in super::commands::all() {
            let action = (command.action)();
            let bound: Vec<String> = keymap
                .bindings_for_action(action.as_ref())
                .filter_map(|binding| super::commands::binding_of(binding.keystrokes()[0].inner()))
                .collect();
            for (key, _) in command.defaults {
                let key = super::commands::normalized(key);
                assert!(bound.contains(&key), "{} is not bound to {key}", command.id);
            }
        }
    });

    // Recording takes the key without running what it was bound to.
    fixture.update(cx, |workspace, window, cx| {
        workspace.on_open_keymap(&super::keymap_view::OpenKeymap, window, cx);
        workspace.start_recording("save", window, cx);
    });
    fixture.press("ctrl-k", cx);
    fixture.update(cx, |workspace, _, cx| workspace.remove_key("save", "ctrl-s".to_string(), cx));
    let keys = fixture.workspace.read_with(cx, |workspace, _| workspace.app.keymap.get("save").cloned());
    assert_eq!(keys, Some(vec!["ctrl-k".to_string()]));

    fixture.press("escape", cx);
    fixture.set_cursor(0, 12, cx);
    fixture.input("\n", cx);
    fixture.press("ctrl-s", cx);
    assert!(fixture.dirty(cx));
    fixture.press("ctrl-k", cx);
    assert!(!fixture.dirty(cx));

    // Back to the defaults.
    fixture.update(cx, |workspace, _, cx| workspace.reset_keymap(cx));
    fixture.input("\n", cx);
    fixture.press("ctrl-k", cx);
    assert!(fixture.dirty(cx));
    fixture.press("ctrl-s", cx);
    assert!(!fixture.dirty(cx));
}

#[gpui_kit::test]
fn counts_after_declarations_open_the_usages_list(cx: &mut TestAppContext) {
    let port = "package port\n\ntype Store interface {\n\tLoad(id string) (int64, error)\n}\n\nfunc Use(s Store) {\n\ts.Load(\"a\")\n}\n";
    let fixture = Fixture::new(
        cx,
        "lens",
        &[
            GO_MOD,
            ("port/port.go", port),
            (
                "infra/client.go",
                "package infra\n\ntype Client struct{}\n\nfunc (c *Client) Load(id string) (int64, error) { return 0, nil }\n",
            ),
        ],
        "port/port.go",
    );
    let notes = |cx: &mut TestAppContext| {
        cx.executor().advance_clock(std::time::Duration::from_millis(300));
        fixture.settle(cx);
        fixture.workspace.read_with(cx, |workspace, _| {
            let active = workspace.active.unwrap();
            workspace
                .lens_notes(active)
                .into_iter()
                .map(|(row, _, note)| (row, note.to_string()))
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        notes(cx),
        [
            (2, "1 个实现 · 1 处用法".to_string()),
            (3, "1 个实现 · 1 处用法".to_string()),
            (6, "无用法".to_string()),
        ]
    );

    // A click on the note lists the single use instead of going there.
    let end = port.find("error)").unwrap() + "error)".len();
    let at = fixture.point_of(end..end, cx);
    fixture.click(gpui_kit::point(at.x + px(60.), at.y), None, gpui_kit::Modifiers::default(), cx);
    assert_eq!(palette_rows(&fixture, cx), [("infra/client.go".to_string(), 4, "实现"), ("port/port.go".to_string(), 7, "")]);

    // A new use counts once the file is saved.
    fixture.press("escape", cx);
    fixture.set_cursor(8, 0, cx);
    fixture.input("\nfunc again(s Store) { s.Load(\"b\") }\n", cx);
    fixture.press("ctrl-s", cx);
    assert!(notes(cx).contains(&(3, "1 个实现 · 2 处用法".to_string())));
}

#[gpui_kit::test]
fn a_chosen_theme_is_kept_for_its_mode(cx: &mut TestAppContext) {
    let fixture = Fixture::new(cx, "themes", &[GO_MOD, ("main.go", "package main\n")], "main.go");
    let name = |cx: &mut TestAppContext| {
        cx.update(|cx| gpui_kit::component::ActiveTheme::theme(cx).theme_name().to_string())
    };
    cx.update(|cx| {
        crate::theme::init(gpui_kit::component::ThemeMode::Dark, "Gust Dark", "Gust Light", cx)
    });
    assert_eq!(name(cx), "Gust Dark");

    fixture.update(cx, |workspace, _, cx| {
        workspace.change_app_settings(cx, |app| {
            app.dark = true;
            app.dark_theme = "Nord".to_string();
        })
    });
    assert_eq!(name(cx), "Nord");
    let blue = cx.update(|cx| crate::theme::palette(cx).blue);
    assert_eq!(blue, gpui_kit::rgb(0x81a1c1).into());

    fixture.update(cx, |workspace, _, cx| {
        workspace.change_app_settings(cx, |app| {
            app.dark = false;
            app.light_theme = "Solarized Light".to_string();
        })
    });
    assert_eq!(name(cx), "Solarized Light");

    // The theme button goes back to the dark theme chosen before.
    fixture.update(cx, |workspace, window, cx| workspace.on_toggle_theme(&super::ToggleTheme, window, cx));
    assert_eq!(name(cx), "Nord");

    fixture.update(cx, |workspace, _, cx| {
        workspace.change_app_settings(cx, |app| *app = crate::settings::AppSettings::default())
    });
}

#[gpui_kit::test]
fn brackets_follow_edits_and_pair_up_at_the_caret(cx: &mut TestAppContext) {
    let source = "package main\n\nfunc main() { f(a[1]) }\n";
    let fixture = Fixture::new(cx, "brackets", &[GO_MOD, ("main.go", source)], "main.go");
    let scan = |cx: &mut TestAppContext| {
        fixture.workspace.read_with(cx, |workspace, _| {
            workspace.docs[workspace.active.unwrap()].brackets.scan.clone()
        })
    };
    assert_eq!(scan(cx).all.len(), 8);
    let open = source.find("f(").unwrap() + 1;
    let close = source.find("])").unwrap() + 1;
    assert_eq!(scan(cx).pair_at(open), Some((open, close)));

    // A bracket typed in a string is not one.
    fixture.set_cursor(2, 14, cx);
    fixture.input("s := \"(\"; ", cx);
    assert_eq!(scan(cx).all.len(), 8);
    assert!(scan(cx).all.iter().all(|bracket| bracket.depth.is_some()));
}

#[gpui_kit::test]
fn a_worktree_is_made_listed_and_removed_from_its_panel(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "worktrees", &[("main.go", "package main\n"), (".gitignore", ".env\n")], "main.go");
    std::fs::write(fixture.root.join(".env"), "PORT=8080\n").unwrap();
    fixture.settle(cx);
    let count = |cx: &mut TestAppContext| fixture.workspace.read_with(cx, |workspace, _| workspace.worktrees.list.len());
    assert_eq!(count(cx), 1);

    fixture.update(cx, |workspace, window, cx| workspace.open_new_worktree(window, cx));
    fixture.input("feature/x", cx);
    let place = fixture.workspace.read_with(cx, |workspace, cx| {
        let dialog = workspace.worktrees.dialog.as_ref().expect("the box is open");
        PathBuf::from(dialog.location.read(cx).value().to_string())
    });
    // Beside the repository, in a folder named after it, however the temp folder is spelled.
    let beside = fixture.root.parent().unwrap().join(format!("{}.worktrees", fixture.root.file_name().unwrap().to_string_lossy()));
    assert_eq!(place.file_name().unwrap(), "feature-x");
    assert_eq!(place.parent().unwrap().file_name(), beside.file_name());
    assert!(crate::worktree::same_path(place.parent().unwrap().parent().unwrap(), fixture.root.parent().unwrap()));
    fixture.update(cx, |workspace, window, cx| {
        workspace.worktrees.dialog.as_mut().unwrap().new_window = false;
        workspace.create_worktree(window, cx);
    });
    fixture.settle(cx);
    fixture.update(cx, |workspace, _, cx| workspace.refresh_git(cx));
    fixture.settle(cx);
    assert_eq!(count(cx), 2);
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.worktrees.dialog.is_none()));
    assert_eq!(std::fs::read_to_string(place.join(".env")).unwrap(), "PORT=8080\n");

    // Removing it asks first, then takes the folder away.
    fixture.update(cx, |workspace, window, cx| workspace.ask_remove_worktree(place.clone(), window, cx));
    fixture.press("enter", cx);
    fixture.settle(cx);
    assert!(!place.exists());
    fixture.update(cx, |workspace, _, cx| workspace.refresh_git(cx));
    fixture.settle(cx);
    assert_eq!(count(cx), 1);
    let _ = std::fs::remove_dir_all(&beside);
}

#[gpui_kit::test]
fn a_worktree_is_reviewed_reverted_merged_and_cleaned_up(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "worktree-merge", &[("a.txt", "one\ntwo\nthree\n")], "a.txt");
    let place = fixture.root.parent().unwrap().join(format!("{}.worktrees", fixture.root.file_name().unwrap().to_string_lossy())).join("work");
    git(&fixture.root, &["worktree", "add", "--quiet", "-b", "work", &place.to_string_lossy(), "main"]);
    std::fs::write(place.join("a.txt"), "ONE\ntwo\nTHREE\n").unwrap();
    git(&place, &["commit", "--quiet", "-am", "shout"]);
    std::fs::write(place.join("b.txt"), "new\n").unwrap();
    fixture.update(cx, |workspace, _, cx| workspace.refresh_git(cx));
    fixture.settle(cx);

    // The review lists committed and uncommitted work alike.
    fixture.update(cx, |workspace, window, cx| workspace.review_worktree(place.clone(), window, cx));
    fixture.settle(cx);
    let listed = fixture.workspace.read_with(cx, |workspace, _| {
        let tab = workspace.diff.as_ref().expect("the review is open");
        tab.entries.iter().map(|entry| (entry.path.clone(), entry.status)).collect::<Vec<_>>()
    });
    assert_eq!(listed, [("a.txt".to_string(), 'M'), ("b.txt".to_string(), 'A')]);

    // Taking back the last change leaves the first.
    fixture.update(cx, |workspace, _, cx| workspace.revert_worktree_chunk(1, cx));
    fixture.settle(cx);
    assert_eq!(std::fs::read_to_string(place.join("a.txt")).unwrap(), "ONE\ntwo\nthree\n");

    // Merging commits what was left uncommitted, then squashes it all onto main.
    fixture.update(cx, |workspace, window, cx| workspace.open_merge_worktree(place.clone(), window, cx));
    let (strategy, commit_first) = fixture.workspace.read_with(cx, |workspace, _| {
        let dialog = workspace.worktrees.merging.as_ref().expect("the merge box is open");
        (dialog.strategy, dialog.commit_first)
    });
    assert_eq!(strategy, crate::worktree::Strategy::Squash);
    assert!(commit_first);
    fixture.update(cx, |workspace, window, cx| workspace.merge_worktree(window, cx));
    fixture.settle(cx);
    assert_eq!(std::fs::read_to_string(fixture.root.join("a.txt")).unwrap(), "ONE\ntwo\nthree\n");
    assert_eq!(std::fs::read_to_string(fixture.root.join("b.txt")).unwrap(), "new\n");
    assert_eq!(git(&fixture.root, &["log", "-1", "--format=%s"]).trim(), "shout");
    assert!(fixture.workspace.read_with(cx, |workspace, _| workspace.diff.is_none()), "the review closes");

    // Then it offers to take the worktree and its branch away.
    fixture.press("enter", cx);
    fixture.settle(cx);
    assert!(!place.exists());
    assert!(!git(&fixture.root, &["branch"]).contains("work"));
    let _ = std::fs::remove_dir_all(place.parent().unwrap());
}

#[gpui_kit::test]
fn files_compare_across_worktrees_and_new_ones_get_their_own_settings(cx: &mut TestAppContext) {
    let fixture = Fixture::in_repository(cx, "worktree-across", &[("a.txt", "one\ntwo\n")], "a.txt");
    let beside = fixture.root.parent().unwrap().join(format!("{}.worktrees", fixture.root.file_name().unwrap().to_string_lossy()));
    let other = beside.join("other");
    git(&fixture.root, &["worktree", "add", "--quiet", "-b", "other", &other.to_string_lossy(), "main"]);
    std::fs::write(other.join("a.txt"), "one\nTWO\n").unwrap();
    fixture.update(cx, |workspace, _, cx| workspace.refresh_git(cx));
    fixture.settle(cx);

    // The open file against its copy in the other worktree; a click takes its lines.
    let names: Vec<String> = fixture.workspace.read_with(cx, |workspace, _| {
        workspace.other_worktrees().into_iter().map(|(name, _)| name).collect()
    });
    assert_eq!(names, ["other"]);
    let file = fixture.root.join("a.txt");
    fixture.update(cx, |workspace, window, cx| {
        workspace.compare_with_worktree(&file, &other, "other".to_string(), window, cx)
    });
    fixture.settle(cx);
    fixture.update(cx, |workspace, window, cx| workspace.take_disk_chunk(0, window, cx));
    assert_eq!(fixture.text(cx), "one\nTWO\n");

    // A new worktree's run configurations take the variables given for it.
    fixture.update(cx, |workspace, _, _| {
        workspace.settings.runs = vec![crate::run::RunConfig { env: "PORT=8080\nAPP=dev".into(), ..Default::default() }];
        crate::settings::save(&workspace.root, &workspace.settings).unwrap();
    });
    fixture.update(cx, |workspace, window, cx| workspace.open_new_worktree(window, cx));
    fixture.input("second", cx);
    let second = fixture.workspace.read_with(cx, |workspace, cx| {
        PathBuf::from(workspace.worktrees.dialog.as_ref().unwrap().location.read(cx).value().to_string())
    });
    fixture.update(cx, |workspace, window, cx| {
        let dialog = workspace.worktrees.dialog.as_mut().unwrap();
        dialog.new_window = false;
        dialog.env.update(cx, |input, cx| input.set_value("PORT=8081", window, cx));
        workspace.create_worktree(window, cx);
    });
    fixture.settle(cx);
    let runs = crate::settings::load(&second).runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].env, "APP=dev\nPORT=8081");

    // Its marker takes the color chosen for it.
    fixture.update(cx, |workspace, _, cx| workspace.set_worktree_color(other.clone(), "green", cx));
    let (marker, green) = fixture.workspace.read_with(cx, |workspace, cx| {
        (workspace.worktree_color(&other, cx), crate::theme::palette(cx).green)
    });
    assert_eq!(marker, green);
    let _ = std::fs::remove_dir_all(&beside);
}
