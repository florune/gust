//! Local history: earlier versions of files, kept outside the project before
//! they are overwritten, so that changes not yet committed can be recovered.
//!
//! Each version is a JSON file under the configuration folder, grouped by
//! project and by file. A file keeps its latest versions for a limited time.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Versions kept per file, and for how long.
const KEEP: usize = 50;
const KEEP_DAYS: i64 = 30;
/// Larger files are not kept.
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Stored {
    path: String,
    time: i64,
    label: String,
    text: String,
}

/// One kept version, without its text.
#[derive(Clone, Debug, PartialEq)]
pub struct Version {
    /// Where the version is stored.
    pub id: PathBuf,
    /// Seconds since the Unix epoch.
    pub time: i64,
    pub label: String,
    pub lines: usize,
}

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ byte as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

fn key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

/// The folder holding the versions of `file` in the project at `root`.
fn folder(base: &Path, root: &Path, file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    base.join(format!("{:016x}", fnv(&key(root))))
        .join(format!("{name}-{:016x}", fnv(&key(file))))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Keep `text` as a version of `file`, unless it is the latest version already.
pub fn record_in(base: &Path, root: &Path, file: &Path, text: &str, label: &str) -> std::io::Result<()> {
    if text.len() > MAX_BYTES {
        return Ok(());
    }
    let dir = folder(base, root, file);
    if let Some(latest) = list_in(base, root, file).first()
        && load(&latest.id).as_deref() == Some(text)
    {
        return Ok(());
    }
    std::fs::create_dir_all(&dir)?;
    let millis = chrono::Utc::now().timestamp_millis();
    let stored = Stored {
        path: file.to_string_lossy().into_owned(),
        time: millis / 1000,
        label: label.to_string(),
        text: text.to_string(),
    };
    // Names sort in the order the versions were kept, several in one
    // millisecond included.
    let mut n = 0;
    let mut id = dir.join(format!("{millis:015}-{n:03}.json"));
    while id.exists() {
        n += 1;
        id = dir.join(format!("{millis:015}-{n:03}.json"));
    }
    std::fs::write(&id, serde_json::to_vec(&stored).unwrap_or_default())?;
    prune(&dir);
    Ok(())
}

/// Drop versions beyond the newest `KEEP`, and those older than `KEEP_DAYS`.
fn prune(dir: &Path) {
    let mut versions = read_dir_versions(dir);
    versions.sort_by(|a, b| b.id.cmp(&a.id));
    let oldest = now() - KEEP_DAYS * 86_400;
    for (ix, version) in versions.iter().enumerate() {
        if ix >= KEEP || version.time < oldest {
            let _ = std::fs::remove_file(&version.id);
        }
    }
}

fn read_dir_versions(dir: &Path) -> Vec<Version> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            let stored: Stored = serde_json::from_slice(&std::fs::read(entry.path()).ok()?).ok()?;
            Some(Version {
                id: entry.path(),
                time: stored.time,
                label: stored.label,
                lines: stored.text.lines().count(),
            })
        })
        .collect()
}

/// The kept versions of `file`, newest first.
pub fn list_in(base: &Path, root: &Path, file: &Path) -> Vec<Version> {
    let mut versions = read_dir_versions(&folder(base, root, file));
    versions.sort_by(|a, b| b.id.cmp(&a.id));
    versions
}

/// The text of a kept version.
pub fn load(id: &Path) -> Option<String> {
    let stored: Stored = serde_json::from_slice(&std::fs::read(id).ok()?).ok()?;
    Some(stored.text)
}

/// Where the versions are kept.
pub fn base() -> PathBuf {
    crate::settings::config_dir().join("history")
}

pub fn record(root: &Path, file: &Path, text: &str, label: &str) {
    let _ = record_in(&base(), root, file, text, label);
}

pub fn list(root: &Path, file: &Path) -> Vec<Version> {
    list_in(&base(), root, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_kept_newest_first_without_repeats() {
        let base = std::env::temp_dir().join(format!("gust-local-history-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = Path::new("/work/project");
        let file = root.join("main.go");
        record_in(&base, root, &file, "one\n", "保存前").unwrap();
        record_in(&base, root, &file, "one\n", "保存前").unwrap();
        record_in(&base, root, &file, "two\n", "外部修改前").unwrap();
        let versions = list_in(&base, root, &file);
        assert_eq!(versions.len(), 2);
        assert_eq!(load(&versions[0].id).as_deref(), Some("two\n"));
        assert_eq!(versions[0].label, "外部修改前");
        assert_eq!(load(&versions[1].id).as_deref(), Some("one\n"));
        // Another file of the same project has its own versions.
        assert!(list_in(&base, root, &root.join("other.go")).is_empty());

        for n in 0..KEEP + 5 {
            record_in(&base, root, &file, &format!("{n}\n"), "保存前").unwrap();
        }
        assert_eq!(list_in(&base, root, &file).len(), KEEP);
        let _ = std::fs::remove_dir_all(&base);
    }
}
