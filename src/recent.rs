//! Projects opened before, newest first, for the project menu.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const LIMIT: usize = 15;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    /// When it was last opened, in seconds since the Unix epoch.
    pub opened: i64,
}

impl RecentProject {
    pub fn name(&self) -> String {
        Path::new(&self.path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.clone())
    }
}

/// Where the list may be kept, in order of preference. The user's own
/// configuration folder can be read-only, as inside a sandbox, so there are
/// fallbacks; reading looks in all of them.
fn locations() -> Vec<PathBuf> {
    let mut dirs = vec![crate::settings::config_dir()];
    dirs.extend(crate::settings::legacy_config_dir());
    if !cfg!(test) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Gust"));
        }
        dirs.push(std::env::temp_dir().join("Gust"));
    }
    dirs.into_iter().map(|dir| dir.join("recent.json")).collect()
}

fn same_path(a: &str, b: &str) -> bool {
    let normal = |path: &str| path.replace('\\', "/").trim_end_matches('/').to_lowercase();
    normal(a) == normal(b)
}

/// Merge lists read from several places: one entry per folder, its latest opening.
fn merge(lists: impl IntoIterator<Item = Vec<RecentProject>>) -> Vec<RecentProject> {
    let mut merged: Vec<RecentProject> = Vec::new();
    for entry in lists.into_iter().flatten() {
        match merged.iter_mut().find(|known| same_path(&known.path, &entry.path)) {
            Some(known) => known.opened = known.opened.max(entry.opened),
            None => merged.push(entry),
        }
    }
    merged.sort_by(|a, b| b.opened.cmp(&a.opened));
    merged
}

/// The project opened most recently, if its folder still exists.
pub fn last() -> Option<PathBuf> {
    load_from(&locations()).first().map(|project| PathBuf::from(&project.path))
}

/// The projects opened before whose folders still exist, newest first.
fn load_from(files: &[PathBuf]) -> Vec<RecentProject> {
    let lists = files.iter().map(|file| {
        std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<RecentProject>>(&text).ok())
            .unwrap_or_default()
    });
    let mut projects: Vec<RecentProject> = merge(lists)
        .into_iter()
        .filter(|project| Path::new(&project.path).is_dir())
        .collect();
    projects.truncate(LIMIT);
    projects
}

/// Note that `root` was opened now, and return the list as it is after.
pub fn record(root: &Path) -> Vec<RecentProject> {
    record_in(&locations(), root)
}

fn record_in(files: &[PathBuf], root: &Path) -> Vec<RecentProject> {
    let path = root.to_string_lossy().into_owned();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    let mut projects = load_from(files);
    projects.retain(|project| !same_path(&project.path, &path));
    projects.insert(0, RecentProject { path, opened: now });
    projects.truncate(LIMIT);
    let text = serde_json::to_string_pretty(&projects).expect("the list serializes");
    for file in files {
        let written = file
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(file, &text));
        if written.is_ok() {
            break;
        }
    }
    projects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_moves_a_project_to_the_top_once() {
        let base = std::env::temp_dir().join(format!("gust-recent-{}", std::process::id()));
        let (a, b) = (base.join("a"), base.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        // Its own list: other tests record the projects they open.
        let list = [base.join("recent.json")];
        record_in(&list, &a);
        record_in(&list, &b);
        let names: Vec<String> = record_in(&list, &a).iter().map(RecentProject::name).collect();
        assert_eq!(&names[..2], ["a", "b"]);
        assert_eq!(names.iter().filter(|name| *name == "a").count(), 1);

        // A folder that is gone is not offered.
        std::fs::remove_dir_all(&b).unwrap();
        assert!(load_from(&list).iter().all(|project| project.name() != "b"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn lists_from_several_places_keep_each_folder_once_at_its_latest() {
        let entry = |path: &str, opened| RecentProject { path: path.into(), opened };
        let merged = merge([
            vec![entry(r"D:\work\a", 5), entry(r"D:\work\b", 9)],
            vec![entry("d:/work/A/", 7)],
        ]);
        assert_eq!(merged, [entry(r"D:\work\b", 9), entry(r"D:\work\a", 7)]);
    }
}
