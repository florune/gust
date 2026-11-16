//! Every command with a name: what Find Action lists and what the keymap
//! page lets the user rebind. Each has its default keys, the context they
//! are bound in, and the action it dispatches.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui_kit::*;

/// The context of keys that work anywhere in the window.
pub(super) const WORKSPACE: &str = "Workspace";
/// The context of keys that act on the text of an editor.
pub(super) const EDITOR: &str = "Input && mode == editor";

pub(super) struct Command {
    pub id: &'static str,
    pub label: &'static str,
    /// Default keys and the context each is bound in.
    pub defaults: &'static [(&'static str, &'static str)],
    pub action: fn() -> Box<dyn Action>,
}

impl Command {
    /// The contexts the command's keys belong in.
    fn contexts(&self) -> Vec<&'static str> {
        let mut contexts: Vec<&str> = self.defaults.iter().map(|(_, context)| *context).collect();
        contexts.sort_unstable();
        contexts.dedup();
        if contexts.is_empty() {
            contexts.push(WORKSPACE);
        }
        contexts
    }

    /// The keys in effect, given the user's own keymap.
    pub fn keys(&self, keymap: &BTreeMap<String, Vec<String>>) -> Vec<String> {
        match keymap.get(self.id) {
            Some(keys) => keys.clone(),
            None => {
                let mut keys: Vec<String> = Vec::new();
                for (key, _) in self.defaults {
                    let key = normalized(key);
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
                keys
            }
        }
    }
}

macro_rules! command {
    ($id:literal, $label:literal, [$(($key:literal, $context:expr)),*], $action:expr) => {
        Command {
            id: $id,
            label: $label,
            defaults: &[$(($key, $context)),*],
            action: || Box::new($action),
        }
    };
}

pub(super) fn all() -> Vec<Command> {
    use super::action_search::*;
    use super::editing::*;
    use super::intentions::ShowIntentions;
    use super::keymap_view::OpenKeymap;
    use super::navigation::*;
    use super::outline::ShowOutline;
    use super::palette::{FindFile, FindSymbol, FindText, ReplaceText};
    use super::problems_view::{NextProblem, PreviousProblem, ShowProblems};
    use super::quick_doc::QuickDocumentation;
    use super::rename_symbol::RenameSymbol;
    use super::rollback::RollbackLines;
    use super::run_panel::{BuildProgram, EditRunConfigs, RunProgram, StopProgram};
    use super::selection::{ExtendSelection, ShrinkSelection};
    use super::settings_view::OpenSettings;
    use super::split::{CloseSplit, SplitRight};
    use super::test_runner::RunTestAtCaret;
    use super::usages::FindUsages;
    use super::*;
    use gpui_kit::component::input::{GoToDefinition, ShowCompletions};
    const W: &str = WORKSPACE;
    const E: &str = EDITOR;
    vec![
        command!("find_file", "查找文件", [("ctrl-p", W), ("ctrl-shift-n", W)], FindFile),
        command!("find_text", "全局搜索", [("ctrl-shift-f", W)], FindText),
        command!("replace_text", "全局替换", [("ctrl-shift-r", W)], ReplaceText),
        command!("find_symbol", "查找符号", [("ctrl-n", W), ("ctrl-alt-shift-n", W)], FindSymbol),
        command!("recent_files", "最近文件", [("ctrl-e", E), ("ctrl-e", W)], RecentFiles),
        command!("find_action", "操作搜索", [("ctrl-shift-a", W)], FindAction),
        command!("outline", "文件结构", [("ctrl-f12", W), ("ctrl-alt-o", W)], ShowOutline),
        command!("go_to_definition", "跳转到定义", [("ctrl-b", E), ("f12", E)], GoToDefinition),
        command!("find_usages", "查找用法", [("alt-f7", E), ("ctrl-alt-b", E)], FindUsages),
        command!("quick_doc", "快速文档", [("ctrl-q", E)], QuickDocumentation),
        command!("rename_symbol", "重命名符号", [("shift-f6", E)], RenameSymbol),
        command!("intentions", "意图操作", [("alt-enter", E)], ShowIntentions),
        command!("completion", "补全", [("ctrl-space", E)], ShowCompletions),
        command!("navigate_back", "后退", [("ctrl-alt-left", W)], NavigateBack),
        command!("navigate_forward", "前进", [("ctrl-alt-right", W)], NavigateForward),
        command!("extend_selection", "扩大选区", [("ctrl-w", E)], ExtendSelection),
        command!("shrink_selection", "缩小选区", [("ctrl-shift-w", E)], ShrinkSelection),
        command!("toggle_comment", "注释 / 取消注释", [("ctrl-/", E)], ToggleComment),
        command!("duplicate_line", "复制行", [("ctrl-d", E)], DuplicateLine),
        command!("delete_line", "删除行", [("ctrl-y", E)], DeleteLine),
        command!("move_line_up", "上移行", [("ctrl-shift-up", E), ("alt-shift-up", E)], MoveLineUp),
        command!("move_line_down", "下移行", [("ctrl-shift-down", E), ("alt-shift-down", E)], MoveLineDown),
        command!("save", "保存", [("ctrl-s", W)], Save),
        command!("format", "格式化", [("ctrl-alt-l", W), ("shift-alt-f", W)], Format),
        command!("close_tab", "关闭标签", [("ctrl-f4", W)], CloseTab),
        command!("split_right", "向右拆分", [], SplitRight),
        command!("close_split", "关闭拆分", [], CloseSplit),
        command!("show_problems", "问题列表", [("alt-6", W)], ShowProblems),
        command!("next_problem", "下一个问题", [("f2", W)], NextProblem),
        command!("previous_problem", "上一个问题", [("shift-f2", W)], PreviousProblem),
        command!("run", "运行", [("shift-f10", W)], RunProgram),
        command!("build", "编译", [("ctrl-f9", W)], BuildProgram),
        command!("stop", "停止", [("ctrl-f2", W)], StopProgram),
        command!("run_test", "运行测试", [("ctrl-shift-f10", W)], RunTestAtCaret),
        command!("edit_run_configs", "编辑运行配置", [], EditRunConfigs),
        command!("go_mod_tidy", "go mod tidy", [], GoModTidy),
        command!("rollback_lines", "回滚光标处的改动", [("ctrl-alt-z", E)], RollbackLines),
        command!("local_history", "本地历史", [], ShowLocalHistory),
        command!("commit", "提交", [("alt-0", W)], ShowCommit),
        command!("git_log", "Git 日志", [("alt-9", W)], ShowGitLog),
        command!("new_branch", "新建分支", [], NewBranch),
        command!("stash", "暂存改动 (stash)", [], StashChanges),
        command!("unstash", "恢复暂存", [], UnstashChanges),
        command!("terminal", "终端", [("alt-f12", W), ("ctrl-`", W)], ToggleTerminal),
        command!("new_terminal", "新建终端", [("ctrl-shift-`", W)], NewTerminal),
        command!("sidebar", "项目面板", [("alt-1", W)], ToggleSidebar),
        command!("open_folder", "打开文件夹", [("ctrl-o", W)], OpenFolder),
        command!("toggle_theme", "切换浅色 / 深色", [], ToggleTheme),
        command!("settings", "设置", [("ctrl-alt-s", W)], OpenSettings),
        command!("keymap", "快捷键", [], OpenKeymap),
    ]
}

impl super::Workspace {
    /// The first key of command `id` as people read it, or nothing when it has none.
    pub(super) fn shortcut(&self, id: &str) -> String {
        all()
            .into_iter()
            .find(|command| command.id == id)
            .and_then(|command| command.keys(&self.app.keymap).first().map(|key| display_key(key)))
            .unwrap_or_default()
    }

    /// A tooltip naming what a control does, and its key if it has one.
    pub(super) fn tip(&self, label: &str, id: &str) -> SharedString {
        match self.shortcut(id) {
            key if key.is_empty() => label.to_string().into(),
            key => format!("{label} ({key})").into(),
        }
    }
}

/// A key as written in bindings, `ctrl-shift-f`, as people read it: `Ctrl+Shift+F`.
pub(super) fn display_key(key: &str) -> String {
    // A trailing `-` is the minus key itself.
    let (rest, last) = match key.strip_suffix("--") {
        Some(rest) => (rest, "-"),
        None => match key.rsplit_once('-') {
            Some((rest, last)) => (rest, last),
            None => ("", key),
        },
    };
    let mut parts: Vec<String> = rest
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|modifier| match modifier {
            "ctrl" => "Ctrl".to_string(),
            "alt" => "Alt".to_string(),
            "shift" => "Shift".to_string(),
            "cmd" | "win" | "super" => "Win".to_string(),
            other => other.to_string(),
        })
        .collect();
    let key = match last {
        "left" => "←".to_string(),
        "right" => "→".to_string(),
        "up" => "↑".to_string(),
        "down" => "↓".to_string(),
        "enter" => "Enter".to_string(),
        "space" => "Space".to_string(),
        "escape" => "Esc".to_string(),
        "tab" => "Tab".to_string(),
        "backspace" => "Backspace".to_string(),
        "delete" => "Delete".to_string(),
        other if other.len() == 1 => other.to_uppercase(),
        other => {
            let mut chars = other.chars();
            chars.next().map_or(String::new(), |first| first.to_uppercase().chain(chars).collect())
        }
    };
    parts.push(key);
    parts.join("+")
}

/// A keystroke as written in bindings, or `None` for a modifier alone.
pub(super) fn binding_of(keystroke: &Keystroke) -> Option<String> {
    let key = keystroke.key.as_str();
    if matches!(key, "control" | "ctrl" | "alt" | "shift" | "platform" | "cmd" | "win" | "function" | "fn" | "") {
        return None;
    }
    let modifiers = keystroke.modifiers;
    let mut parts = Vec::new();
    if modifiers.control {
        parts.push("ctrl");
    }
    if modifiers.alt {
        parts.push("alt");
    }
    if modifiers.shift {
        parts.push("shift");
    }
    if modifiers.platform {
        parts.push("win");
    }
    parts.push(key);
    Some(parts.join("-"))
}

/// A key in the one spelling [`binding_of`] gives, so equal keys compare equal.
pub(super) fn normalized(key: &str) -> String {
    Keystroke::parse_with_os(key, OperatingSystem::Windows)
        .ok()
        .and_then(|keystroke| binding_of(&keystroke))
        .unwrap_or_else(|| key.to_string())
}

/// Bind the keys the user chose, where they differ from `previous`. Keys a
/// command had before stop running it; a command left out gets its defaults.
pub(super) fn apply_keymap(
    keymap: &BTreeMap<String, Vec<String>>,
    previous: &BTreeMap<String, Vec<String>>,
    cx: &mut App,
) {
    let mut bindings = Vec::new();
    for command in all() {
        if keymap.get(command.id) == previous.get(command.id) {
            continue;
        }
        let keys = command.keys(keymap);
        let name = SharedString::from((command.action)().name().to_string());
        let mut old: Vec<String> = command.defaults.iter().map(|(key, _)| normalized(key)).collect();
        old.extend(previous.get(command.id).into_iter().flatten().cloned());
        old.sort();
        old.dedup();
        // An unbinding outranks only the bindings of its own context added before it.
        for context in command.contexts() {
            for key in old.iter().filter(|key| !keys.contains(key)) {
                if let Some(binding) = load(key, Box::new(Unbind(name.clone())), context, cx) {
                    bindings.push(binding);
                }
            }
        }
        if keymap.contains_key(command.id) {
            for context in command.contexts() {
                for key in &keys {
                    if let Some(binding) = load(key, (command.action)(), context, cx) {
                        bindings.push(binding);
                    }
                }
            }
        } else {
            for (key, context) in command.defaults {
                if let Some(binding) = load(key, (command.action)(), context, cx) {
                    bindings.push(binding);
                }
            }
        }
    }
    cx.bind_keys(bindings);
}

fn load(key: &str, action: Box<dyn Action>, context: &str, cx: &App) -> Option<KeyBinding> {
    let predicate = KeyBindingContextPredicate::parse(context).ok()?;
    KeyBinding::load(key, action, Some(Rc::new(predicate)), false, None, cx).ok()
}

#[cfg(test)]
mod tests {
    use super::{all, display_key, normalized};

    #[test]
    fn keys_read_as_people_write_them() {
        assert_eq!(display_key("ctrl-shift-f"), "Ctrl+Shift+F");
        assert_eq!(display_key("ctrl-alt-left"), "Ctrl+Alt+←");
        assert_eq!(display_key("f12"), "F12");
        assert_eq!(display_key("ctrl-/"), "Ctrl+/");
        assert_eq!(display_key("ctrl--"), "Ctrl+-");
    }

    #[test]
    fn modifiers_in_any_order_are_the_same_key() {
        assert_eq!(normalized("shift-alt-f"), "alt-shift-f");
        assert_eq!(normalized("ctrl-shift-`"), "ctrl-shift-`");
    }

    #[test]
    fn every_command_has_its_own_name() {
        let mut ids: Vec<&str> = all().iter().map(|command| command.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count);
    }
}
