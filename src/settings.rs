//! Per-project settings, kept outside the project in the user's own
//! configuration folder, so nothing is added to the repository.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    /// The project folder these settings belong to, for anyone reading the file.
    pub root: String,
    /// Files and folders left out of the file tree, the file finder, and
    /// text search: paths from the project root, with `/` separators.
    pub hidden: Vec<String>,
    /// Run configurations, and which one the run button starts.
    pub runs: Vec<crate::run::RunConfig>,
    pub selected_run: usize,
    /// What was open when the project was last left.
    pub session: Session,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    /// The open files in tab order, from the project root.
    pub open: Vec<OpenFile>,
    pub active: Option<String>,
    /// Files opened last, the latest first, for Ctrl+E.
    pub recent: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenFile {
    pub path: String,
    /// The caret: zero-based line and byte column.
    pub row: u32,
    pub col: u32,
}

/// Whether `relative` is one of `hidden` or lies inside one of them.
pub fn is_hidden(hidden: &[String], relative: &str) -> bool {
    hidden.iter().any(|entry| {
        relative == entry
            || relative
                .strip_prefix(entry.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// Where Gust keeps its own files: `.gust` in the user's home folder.
///
/// Not `%APPDATA%`: Windows gives a program started from inside a packaged
/// app (such as an AI tool) its own private copy of that folder, so Gust
/// started from there and Gust started from Explorer would each keep a list
/// the other cannot see.
pub fn config_dir() -> PathBuf {
    if cfg!(test) {
        return std::env::temp_dir().join(format!("gust-test-config-{}", std::process::id()));
    }
    if let Some(dir) = std::env::var_os("GUST_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" });
    home.map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(".gust")
}

/// Where earlier versions kept their files; still read, never written.
pub fn legacy_config_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
    };
    base.map(|base| base.join("Gust"))
}

/// The settings file of the project at `root`: one per folder, named by a
/// hash of its path so that every folder gets its own.
fn settings_file(root: &Path) -> PathBuf {
    settings_file_in(&config_dir(), root)
}

fn settings_file_in(config: &Path, root: &Path) -> PathBuf {
    let key = root.to_string_lossy().replace('\\', "/").to_lowercase();
    // FNV-1a: stable across runs and Rust versions, unlike the std hasher.
    let hash = key
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
            (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3)
        });
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    config
        .join("projects")
        .join(format!("{name}-{hash:016x}.json"))
}

/// The folder inside a project that holds its settings when the user's own
/// configuration folder cannot be written to, as inside a sandbox.
pub const LOCAL_DIR: &str = ".gust";

fn local_file(root: &Path) -> PathBuf {
    root.join(LOCAL_DIR).join("settings.json")
}

pub fn load(root: &Path) -> ProjectSettings {
    let file = settings_file(root);
    // Settings saved where earlier versions kept them, until saved again.
    let legacy = legacy_config_dir()
        .map(|dir| settings_file_in(&dir, root))
        .filter(|legacy| !file.is_file() && legacy.is_file());
    load_from(legacy.as_ref().unwrap_or(&file), root)
}

/// Read the project's settings: from inside the project when they were saved
/// there, otherwise from `global`.
fn load_from(global: &Path, root: &Path) -> ProjectSettings {
    let local = local_file(root);
    let file = if local.is_file() { local } else { global.to_path_buf() };
    let mut settings: ProjectSettings = std::fs::read_to_string(file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    settings.root = root.to_string_lossy().into_owned();
    settings
}

pub fn save(root: &Path, settings: &ProjectSettings) -> Result<(), String> {
    save_to(&settings_file(root), root, settings)
}

fn write(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, text)
}

/// Save to `global`, unless the project already keeps its settings itself or
/// `global` cannot be written; then into the project, in a folder git ignores.
fn save_to(global: &Path, root: &Path, settings: &ProjectSettings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(settings).expect("settings serialize");
    let local = local_file(root);
    if !local.is_file() && write(global, &text).is_ok() {
        return Ok(());
    }
    write(&local, &text).map_err(|err| format!("无法保存项目设置：{err}"))?;
    // The folder ignores itself, so it never shows up as a change to commit.
    let ignore = root.join(LOCAL_DIR).join(".gitignore");
    if !ignore.exists() {
        let _ = std::fs::write(ignore, "*\n");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_folder_hides_everything_inside_it_and_nothing_beside_it() {
        let hidden = ["docs".to_string(), "deploy.sh".to_string()];
        assert!(is_hidden(&hidden, "docs"));
        assert!(is_hidden(&hidden, "docs/spec/a.md"));
        assert!(is_hidden(&hidden, "deploy.sh"));
        assert!(!is_hidden(&hidden, "docs2/a.md"));
        assert!(!is_hidden(&hidden, "cmd/docs"));
    }

    #[test]
    fn settings_go_into_the_project_when_the_config_folder_is_locked() {
        let base = std::env::temp_dir().join(format!("gust-locked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("project");
        std::fs::create_dir_all(&root).unwrap();
        // A file where the config folder should be: nothing can be written under it.
        std::fs::write(base.join("locked"), "").unwrap();
        let global = base.join("locked").join("projects").join("p.json");
        let settings = ProjectSettings {
            root: String::new(),
            hidden: vec![".idea".into()],
            ..Default::default()
        };

        save_to(&global, &root, &settings).unwrap();
        assert!(root.join(".gust/settings.json").is_file());
        assert_eq!(std::fs::read_to_string(root.join(".gust/.gitignore")).unwrap(), "*\n");
        assert_eq!(load_from(&global, &root).hidden, [".idea"]);

        // Once the project keeps its settings, they stay there.
        let free = base.join("free.json");
        save_to(&free, &root, &ProjectSettings::default()).unwrap();
        assert!(!free.exists());
        assert!(load_from(&free, &root).hidden.is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn settings_are_kept_per_project() {
        let a = std::env::temp_dir().join("gust-settings-a");
        let b = std::env::temp_dir().join("gust-settings-b");
        let settings = ProjectSettings {
            root: String::new(),
            hidden: vec![".idea".into(), "migrations".into()],
            ..Default::default()
        };
        save(&a, &settings).unwrap();
        assert_eq!(load(&a).hidden, settings.hidden);
        assert!(load(&b).hidden.is_empty());
    }
}
