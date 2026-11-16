//! Find Action (Ctrl+Shift+A): every command by name, with its shortcut.

use gpui_kit::*;

use super::Workspace;
use super::commands::{self, Command, display_key};
use super::palette::Mode;
use crate::finder::Hit;

actions!(gust, [FindAction, GoModTidy, NewBranch, StashChanges, UnstashChanges, ShowLocalHistory]);

pub(super) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-shift-a", FindAction, Some("Workspace"))]);
}

/// What Find Action shows of a command's keys.
fn shown_keys(command: &Command, keymap: &std::collections::BTreeMap<String, Vec<String>>) -> String {
    let mut keys: Vec<String> = command.keys(keymap).iter().map(|key| display_key(key)).collect();
    if command.id == "find_file" {
        keys.insert(0, "Shift Shift".to_string());
    }
    keys.join(" / ")
}

impl Workspace {
    pub(super) fn on_find_action(&mut self, _: &FindAction, window: &mut Window, cx: &mut Context<Self>) {
        let hits: Vec<Hit> = commands::all()
            .iter()
            .enumerate()
            .map(|(ix, command)| Hit {
                path: ix.to_string(),
                row: 0,
                col: 0,
                preview: command.label.to_string(),
                tag: "",
                detail: shown_keys(command, &self.app.keymap),
            })
            .collect();
        // Commands that act on the editor go to the one that had the keyboard.
        self.action_focus = window.focused(cx);
        self.open_palette(Mode::Actions, window, cx);
        if let Some(palette) = &mut self.palette {
            palette.found = hits.clone();
            palette.hits = hits;
        }
        cx.notify();
    }

    /// Run command `ix` of the list, where the keyboard was before.
    pub(super) fn run_command(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(command) = commands::all().into_iter().nth(ix) else {
            return;
        };
        let action = (command.action)();
        if let Some(focus) = self.action_focus.take() {
            focus.focus(window, cx);
        }
        window.defer(cx, move |window, cx| window.dispatch_action(action, cx));
    }

    pub(super) fn on_go_mod_tidy(&mut self, _: &GoModTidy, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .active
            .and_then(|ix| self.docs.get(ix))
            .and_then(|doc| doc.path.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| self.root.clone());
        self.run_go_tool(&dir, &["mod", "tidy"], "go mod tidy", window, cx);
    }

    pub(super) fn on_new_branch(&mut self, _: &NewBranch, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_prompt(super::file_ops::FileAction::NewBranch, window, cx);
    }

    pub(super) fn on_stash_changes(&mut self, _: &StashChanges, window: &mut Window, cx: &mut Context<Self>) {
        self.open_file_prompt(super::file_ops::FileAction::Stash, window, cx);
    }

    pub(super) fn on_unstash_changes(&mut self, _: &UnstashChanges, window: &mut Window, cx: &mut Context<Self>) {
        self.show_stashes(window, cx);
    }

    pub(super) fn on_show_local_history(
        &mut self,
        _: &ShowLocalHistory,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(path) = self.active.and_then(|ix| self.docs.get(ix)).map(|doc| doc.path.clone()) {
            self.show_local_history(path, window, cx);
        }
    }
}
