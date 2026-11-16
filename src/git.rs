//! Git through the `git` command, so credentials, proxies, hooks, and SSH
//! configuration all behave exactly as they do in the user's terminal.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use crate::proc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Modified,
    Added,
    Untracked,
    Deleted,
    Renamed,
    Conflict,
}

impl ChangeKind {
    /// The one-letter mark shown beside the file.
    pub fn letter(self) -> &'static str {
        match self {
            Self::Modified => "M",
            Self::Added => "A",
            Self::Untracked => "U",
            Self::Deleted => "D",
            Self::Renamed => "R",
            Self::Conflict => "C",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Change {
    /// Path from the repository root, with `/` separators.
    pub path: String,
    pub kind: ChangeKind,
    pub added: u32,
    pub removed: u32,
    /// The path before a rename.
    pub old_path: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The repository's top-level directory.
    pub root: PathBuf,
    pub branch: String,
    pub has_upstream: bool,
    pub ahead: u32,
    pub behind: u32,
    /// Everything that differs from the last commit.
    pub changes: Vec<Change>,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = proc::command("git")
        // Paths are printed as they are, not octal-escaped.
        .args(["-c", "core.quotepath=false"])
        // Looking must never take the index lock from a git the user is running.
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(dir)
        // Without a terminal a credential prompt would wait forever.
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("无法运行 git：{err}"))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("git 命令失败");
    Err(reason
        .trim()
        .trim_start_matches("fatal: ")
        .trim_start_matches("error: ")
        .to_string())
}

/// The state of the repository containing `dir`, or `None` outside one.
pub fn status(dir: &Path) -> Option<Status> {
    // The root is worked out from `dir` rather than taken from git, which
    // spells it its own way (long names, other case): paths of open files are
    // matched against it.
    let up = git(dir, &["rev-parse", "--show-cdup"]).ok()?;
    let mut root = dir.to_path_buf();
    for segment in up.trim().split('/').filter(|segment| !segment.is_empty()) {
        if segment == ".." {
            root.pop();
        }
    }
    let output = git(
        &root,
        &[
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
        ],
    )
    .ok()?;
    let mut status = parse_status(&output);

    // Line counts against the last commit; absent in a repository without one.
    if let Ok(numstat) = git(&root, &["diff", "--numstat", "HEAD"]) {
        for line in numstat.lines() {
            let mut fields = line.splitn(3, '\t');
            let (Some(added), Some(removed), Some(path)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if let Some(change) = status.changes.iter_mut().find(|c| c.path == path) {
                change.added = added.parse().unwrap_or(0);
                change.removed = removed.parse().unwrap_or(0);
            }
        }
    }
    for change in &mut status.changes {
        if matches!(change.kind, ChangeKind::Untracked | ChangeKind::Added) && change.added == 0 {
            change.added = count_lines(&root.join(&change.path));
        }
    }
    status.root = root;
    Some(status)
}

/// Lines in a text file small enough to be worth counting.
fn count_lines(path: &Path) -> u32 {
    const LIMIT: u64 = 2 * 1024 * 1024;
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= LIMIT => {}
        _ => return 0,
    }
    std::fs::read(path)
        .ok()
        .filter(|bytes| !bytes.contains(&0))
        .map_or(0, |bytes| {
            let newlines = bytes.iter().filter(|byte| **byte == b'\n').count();
            let unterminated = bytes.last().is_some_and(|last| *last != b'\n');
            (newlines + usize::from(unterminated)) as u32
        })
}

/// Parse `git status --porcelain=v2 --branch -z`.
fn parse_status(output: &str) -> Status {
    let mut status = Status::default();
    let mut entries = output.split('\0');
    while let Some(entry) = entries.next() {
        let (tag, rest) = entry.split_once(' ').unwrap_or((entry, ""));
        match tag {
            "#" => {
                if let Some(head) = rest.strip_prefix("branch.head ") {
                    status.branch = head.to_string();
                } else if rest.starts_with("branch.upstream ") {
                    status.has_upstream = true;
                } else if let Some(counts) = rest.strip_prefix("branch.ab ") {
                    let mut counts = counts.split(' ');
                    let mut next = || {
                        counts
                            .next()
                            .and_then(|count| count[1..].parse().ok())
                            .unwrap_or(0)
                    };
                    status.ahead = next();
                    status.behind = next();
                }
            }
            // `1 XY sub mH mI mW hH hI path`
            "1" => {
                let mut fields = rest.splitn(8, ' ');
                let xy = fields.next().unwrap_or_default();
                let Some(path) = fields.nth(6) else { continue };
                let kind = if xy.contains('D') {
                    ChangeKind::Deleted
                } else if xy.contains('A') {
                    ChangeKind::Added
                } else {
                    ChangeKind::Modified
                };
                status.changes.push(change(path, kind));
            }
            // `2 XY sub mH mI mW hH hI Xscore path`, then the original path.
            "2" => {
                let mut fields = rest.splitn(9, ' ');
                let old_path = entries.next().map(str::to_string);
                if let Some(path) = fields.nth(8) {
                    let mut renamed = change(path, ChangeKind::Renamed);
                    renamed.old_path = old_path;
                    status.changes.push(renamed);
                }
            }
            // `u XY sub m1 m2 m3 mW h1 h2 h3 path`
            "u" => {
                if let Some(path) = rest.splitn(10, ' ').nth(9) {
                    status.changes.push(change(path, ChangeKind::Conflict));
                }
            }
            "?" => status.changes.push(change(rest, ChangeKind::Untracked)),
            _ => {}
        }
    }
    status.changes.sort_by(|a, b| a.path.cmp(&b.path));
    status
}

fn change(path: &str, kind: ChangeKind) -> Change {
    Change {
        path: path.to_string(),
        kind,
        added: 0,
        removed: 0,
        old_path: None,
    }
}

/// The committed text of `path`, or `None` when the last commit has no such file.
pub fn head_text(root: &Path, path: &str) -> Option<String> {
    git(root, &["show", &format!("HEAD:{path}")]).ok()
}

/// Delete a file the last commit does not have: to the Recycle Bin, or for
/// good in tests.
fn remove_new_file(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let removed = if cfg!(test) {
        std::fs::remove_file(path).map_err(|err| err.to_string())
    } else {
        trash::delete(path).map_err(|err| err.to_string())
    };
    removed.map_err(|err| format!("无法删除：{err}"))
}

/// Take `change` back to the last commit, in the index and on disk. A file
/// the last commit does not have is deleted.
pub fn rollback(root: &Path, change: &Change) -> Result<(), String> {
    let restore = |path: &str| {
        git(root, &["restore", "--source=HEAD", "--staged", "--worktree", "--", path]).map(|_| ())
    };
    match change.kind {
        ChangeKind::Untracked => remove_new_file(&root.join(&change.path)),
        ChangeKind::Added => {
            git(root, &["rm", "--cached", "--quiet", "--force", "--", &change.path])?;
            remove_new_file(&root.join(&change.path))
        }
        ChangeKind::Renamed => {
            if let Some(old) = &change.old_path {
                restore(old)?;
            }
            let _ = git(root, &["rm", "--cached", "--quiet", "--force", "--", &change.path]);
            remove_new_file(&root.join(&change.path))
        }
        ChangeKind::Modified | ChangeKind::Deleted | ChangeKind::Conflict => restore(&change.path),
    }
}

/// Commit the working-tree state of `paths`, and nothing else.
pub fn commit(root: &Path, message: &str, paths: &[String]) -> Result<(), String> {
    if message.trim().is_empty() {
        return Err("请填写提交说明".to_string());
    }
    if paths.is_empty() {
        return Err("没有勾选要提交的文件".to_string());
    }
    let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
    // New and deleted files have to be known to the index before a commit
    // limited to paths will take them.
    let mut add = vec!["add", "--all", "--"];
    add.extend(&paths);
    git(root, &add)?;
    let mut commit = vec!["commit", "--quiet", "-m", message, "--"];
    commit.extend(&paths);
    git(root, &commit).map(|_| ())
}

pub fn branches(root: &Path) -> Vec<String> {
    git(root, &["branch", "--format=%(refname:short)"])
        .map(|output| output.lines().map(str::to_string).collect())
        .unwrap_or_default()
}

pub fn switch(root: &Path, branch: &str) -> Result<(), String> {
    git(root, &["switch", branch]).map(|_| ())
}

pub fn pull(root: &Path) -> Result<(), String> {
    git(root, &["pull", "--ff-only"]).map(|_| ())
}

/// Push the current branch, giving it an upstream on its first push.
pub fn push(root: &Path, has_upstream: bool) -> Result<(), String> {
    let args: &[&str] = if has_upstream {
        &["push"]
    } else {
        &["push", "--set-upstream", "origin", "HEAD"]
    };
    git(root, args).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A repository with one commit holding `a.txt` and `b.txt`.
    struct Repo(PathBuf);

    impl Repo {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("gust-git-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).unwrap();
            let repo = Self(root);
            repo.git(&["init", "--quiet", "--initial-branch=main"]);
            repo.git(&["config", "user.email", "test@example.com"]);
            repo.git(&["config", "user.name", "Test"]);
            repo.git(&["config", "core.autocrlf", "false"]);
            repo.write("a.txt", "one\ntwo\nthree\n");
            repo.write("b.txt", "keep\n");
            repo.git(&["add", "."]);
            repo.git(&["commit", "--quiet", "-m", "first"]);
            repo
        }

        fn git(&self, args: &[&str]) {
            git(&self.0, args).unwrap();
        }

        fn write(&self, path: &str, content: &str) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        fn status(&self) -> Status {
            status(&self.0).unwrap()
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_clean_repository_reports_its_branch_and_no_changes() {
        let repo = Repo::new("clean");
        let status = repo.status();
        assert_eq!(status.branch, "main");
        assert!(!status.has_upstream);
        assert!(status.changes.is_empty());
    }

    #[test]
    fn changes_carry_their_kind_and_line_counts() {
        let repo = Repo::new("changes");
        repo.write("a.txt", "one\n2\nthree\nfour\n");
        std::fs::remove_file(repo.0.join("b.txt")).unwrap();
        repo.write("dir/新文件.go", "package dir\n\nfunc F() {}\n");

        let summary: Vec<(String, ChangeKind, u32, u32)> = repo
            .status()
            .changes
            .into_iter()
            .map(|change| (change.path, change.kind, change.added, change.removed))
            .collect();
        assert_eq!(
            summary,
            [
                ("a.txt".to_string(), ChangeKind::Modified, 2, 1),
                ("b.txt".to_string(), ChangeKind::Deleted, 0, 1),
                ("dir/新文件.go".to_string(), ChangeKind::Untracked, 3, 0),
            ]
        );
    }

    #[test]
    fn committing_takes_only_the_chosen_paths() {
        let repo = Repo::new("commit");
        repo.write("a.txt", "changed\n");
        std::fs::remove_file(repo.0.join("b.txt")).unwrap();
        repo.write("new.txt", "fresh\n");

        assert!(commit(&repo.0, "  ", &["a.txt".to_string()]).is_err());
        commit(
            &repo.0,
            "second",
            &["a.txt".to_string(), "new.txt".to_string()],
        )
        .unwrap();

        let left: Vec<(String, ChangeKind)> = repo
            .status()
            .changes
            .into_iter()
            .map(|change| (change.path, change.kind))
            .collect();
        assert_eq!(left, [("b.txt".to_string(), ChangeKind::Deleted)]);
        assert_eq!(head_text(&repo.0, "a.txt").as_deref(), Some("changed\n"));
        assert_eq!(head_text(&repo.0, "missing.txt"), None);
    }

    #[test]
    fn branches_can_be_listed_and_switched() {
        let repo = Repo::new("branches");
        repo.git(&["branch", "feature"]);
        assert_eq!(branches(&repo.0), ["feature", "main"]);
        switch(&repo.0, "feature").unwrap();
        assert_eq!(repo.status().branch, "feature");
        assert!(switch(&repo.0, "nope").is_err());
    }

    #[test]
    fn a_folder_outside_any_repository_has_no_status() {
        let dir = std::env::temp_dir().join(format!("gust-git-none-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // The temp folder itself may sit inside a repository on some machines.
        if git(&dir, &["rev-parse", "--show-toplevel"]).is_err() {
            assert_eq!(status(&dir), None);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_lines_are_parsed_including_renames_and_conflicts() {
        let output = "# branch.oid abc\0# branch.head dev\0# branch.upstream origin/dev\0# branch.ab +2 -1\0\
            1 .M N... 100644 100644 100644 aaa bbb src/a b.go\0\
            2 R. N... 100644 100644 100644 aaa bbb R100 new name.go\0old name.go\0\
            u UU N... 100644 100644 100644 100644 aaa bbb ccc both.go\0\
            ? notes.md\0";
        let status = parse_status(output);
        assert_eq!(status.branch, "dev");
        assert!(status.has_upstream);
        assert_eq!((status.ahead, status.behind), (2, 1));
        let kinds: Vec<(&str, ChangeKind)> = status
            .changes
            .iter()
            .map(|change| (change.path.as_str(), change.kind))
            .collect();
        assert_eq!(
            kinds,
            [
                ("both.go", ChangeKind::Conflict),
                ("new name.go", ChangeKind::Renamed),
                ("notes.md", ChangeKind::Untracked),
                ("src/a b.go", ChangeKind::Modified),
            ]
        );
    }
}
